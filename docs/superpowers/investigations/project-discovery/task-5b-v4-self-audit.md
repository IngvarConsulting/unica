# Task 5B v4 — rejected adversarial contract self-audit

> **REJECTED 2026-07-18 — DO NOT USE AS AN ACCEPTANCE GATE.** Its immutable
> pre-rejection SHA-256 was
> `de13c4509d6333eceb63fde7f70fcd3818ad970fb878fbc6b9e278f5987fdf6d`.
> The audit falsely claimed complete EventSubscription coverage while omitting
> four register-record-set families and ConstantValueManager, and it missed
> result-limit ordering, element-vs-QName namespaces, explicit-Direct,
> Form companion traversal/main-type identity, and self-contained callback/HTTP
> defects. Current conditional authority is `task-5b-contract.md` v5 plus the
> separate v5 self-audit. The original claims below are retained only as evidence
> of the rejected review.

Date: 2026-07-18

Historical original status: **PASS-CONDITIONAL for design; STOP for
implementation — REJECTED by v5 re-audit**.

The v4 contract closes every P0/P1/P2 design finding from the v3 fresh review
and the follow-up CommonModule/runtime-context review. It intentionally does not
close operational G0: no accepted immutable Task 5A commit exists. Task 5B is not
implementation-ready until that SHA exists and the accepted code/spec passes the
section-3 back-propagation audit.

## 1. Immutable audit anchors

| Artifact | SHA-256 / commit |
| --- | --- |
| checkout HEAD observed at audit start | `20f6afa7a09430614babebc0cdeebeb94c8a0189` |
| superseded Task 5B v3 contract | `3062b1d34ba0e93185aee55ca2a3ac05b10b68d6732be039e0db36709535f2cf` |
| superseded Task 5B v3 brief | `03fa268474f5c2937208fce6553837d2d8ffb759de158b94246db05023146d86` |
| v3 fresh review before resolution banner | `cd52d99e2b46c3328443148ec5d7b9be01b92bf22c8216eb37e732379f45e76a` |
| Task 7 before v4 back-propagation | `8e719dde23e61400c965f97c784325f54dfec2320c4c3d0cc8ca71cb37a428fe` |
| **Task 5B v4 contract audited here** | `5c25c74d18b87799e0eea383e9a684d8674b4eefe98cfc2382f5f74fdb2df8bb` |
| **Task 7 after v4 back-propagation** | `dfe521ab491b4696b89728b5ed0089da57eec3320c2af7685c0dced7aef02736` |
| live platform parser observed during audit | `5a361955b9aeb09151e75979b3f9f3c5df5351cd45b6620dfbcef45dcddd02f9` |
| live Form event registry observed during audit | `b0057c23749a2faa2fe0ebc73f6a7929841acfbc4b5afd8d2592370f138c3eea` |

Task 8 was being changed by a concurrent independent reviewer and was not edited
by this task. Its moving content is not an acceptance anchor for this audit. v4
instead states the exact import/back-propagation Task 8 must satisfy after its own
accepted review.

The checkout contained pre-existing/concurrent tracked Task 5A/spec/product
changes. Every edit made by this audit targeted ignored `.superpowers/sdd/*`
documents only. No production test result is claimed.

## 2. Authority cross-check

Primary 1C sources were re-read rather than inferred from v3:

1. EventSubscription requires a non-global CommonModule with exact five-field
   profile `(Global=false, ordinary client=true, managed client=false,
   Server=true, ExternalConnection=true)`, an exported Procedure, and parameter
   count equal to source-event parameters plus Source. It executes in the same
   context as the source action, not necessarily `AtServer`.
2. `ServerCall` is not an EventSubscription validity field. Official 1C material
   contains a ServerCall-enabled handler-module pattern, so v4 neither requires
   false nor true.
3. The general ScheduledJob guide permits an exported Procedure or Function of a
   non-global CommonModule callable on the server. The FAQ's ServerCall=true is an
   example, not a platform-wide predicate. Predefined jobs have no parameters;
   non-predefined runtime instances/parameters are not proven by source metadata.
4. Extension form handlers use current serializer/domain call types and may have
   paired Before/After methods; adopted BaseForm material is distinct from
   extension-local bindings.

Contract sections 2, 3.2-3.3, 8, 9, and 10 encode these conclusions without
inventing defaults.

## 3. v3 finding closure matrix

| Finding | v4 closure | Result |
| --- | --- | --- |
| P0-G0 accepted Task 5A SHA absent | top gate and sections 3.4/15 require clean accepted `TASK5A_ACCEPTED_SHA`; dirty/current HEAD is explicitly inadmissible | **OPEN operational STOP** |
| P0-1 CFE trusts labels/wrapper UUID | sections 3.1/7 require source-bound `BaseOwnedMetadataIdentityV1` and `ExtensionMetadataMembershipV1`; flavor+Own/Adopted are inside whole facts; wrapper UUID never joins | **CLOSED in design** |
| P0-2 local-name-only metadata parser | section 5 requires exact `http://v8.1c.ru/8.3/MDClasses` on every structural/capture node, arbitrary prefix, and fail-closed foreign binding-shaped direct children | **CLOSED in design** |
| P0-3 arbitrary Event identifier | section 8 defines one closed `event-subscriptions/v1` registry, exact source-family/event/signature classes, complete selected-set digest, five-field module profile, and Definition join | **CLOSED in design** |
| P1-1 duplicate/contradictory Form registry | section 10 requires one neutral `platform-form-bindings/v2` registry imported by native form operations, Task 5B, and Task 8; it preserves the full live matrix, Button=no events, BaseForm context, and action policy | **CLOSED in design** |
| P1-2 missing Form gap too narrow | sections 4/6 freeze `FormMaterialScopeV1` with Form, requested FormCommands, exact runtime subjects, and pair key; missing material emits no false command absence | **CLOSED in design** |
| P1-3 result limit lacks material scope | section 6.1 truncates whole semantic evidence groups and maps every dropped group to exact material subjects with fixed 256/2,000 sentinels | **CLOSED in design** |
| P2-1 Task 7 still per-source | Task 7 now has `MetadataComposite`, one Stage-1 call, local internal groups, exact query digest, new RED name, and corrected acceptance item 12 | **CLOSED in design** |

## 4. Follow-up P0 closure

### 4.1 EventSubscription context/capability/signature

PASS:

- Task 5A back-propagation explicitly splits `BindingRuntimeContextV1` from
  `BslExecutionContext` (`task-5b-contract.md:161-204`).
- EventSubscription uses `SameAsSourceEvent`; ScheduledJob/HTTP use Server;
  FormCommand uses Client. None synthesizes a BSL annotation.
- selected sources are nonempty, bounded at 256, exact registered identities,
  canonically unique by set insertion, and digest-bearing
  (`task-5b-contract.md:488-532`). Adjacent-window duplicate detection is
  forbidden.
- the descriptor is the authoritative source set; any `SubscriptionSource`
  projection must derive from it and pass exact-set equality before the
  ExchangePlan mechanism can promote (`task-5b-contract.md:519-527`).
- only BeforeWrite/BeforeDelete closed rows are actionable; mixed incompatible
  arity is Unknown (`task-5b-contract.md:534-553`).
- exact five-field CommonModule profile is material; ServerCall/Privileged are
  non-gating diagnostics without guessed defaults
  (`task-5b-contract.md:555-594`).
- Definition compatibility requires exported Procedure and exact row arity; the
  runtime context remains same-as-source-event.

### 4.2 ScheduledJob capability and instance proof

PASS:

- platform-valid module predicate is exactly Global=false + Server=true; v4 does
  not mistake the FAQ's ServerCall example for a general rule
  (`task-5b-contract.md:598-622`).
- MethodName, Use, and Predefined are separate exact singletons.
- positive v1 runtime root requires Use=true + Predefined=true + exported
  zero-arity Procedure/Function; Function return is ignored.
- Use=true + Predefined=false retains only declarative binding and yields
  `non_predefined_scheduled_job_instance_unproven`, Unknown
  (`task-5b-contract.md:623-662`).
- missing/malformed Predefined is not defaulted.

### 4.3 Form completeness

PASS:

- the complete registry lists every current live target/event family, including
  RadioButtonField, TrackBarField, ExtendedTooltip and document fields;
- Page/Button/CommandBar/AutoCommandBar/UsualGroup/ButtonGroup/Popup are explicit
  no-event targets; Button Events is incomplete;
- persistent Form event and Table DataPath context validation are shared;
- exactly one BaseForm can supply context but its saved bindings are excluded
  from the extension-local projection/audit;
- zero Action is incomplete; regular and borrowed-extension callType/action
  cardinalities are closed and primary-source bounded;
- whole-document unconsumed binding-shaped audit prevents a prefix from proving
  Ordinary/unbound.

## 5. Current live-code gate audit

This section explains why PASS-CONDITIONAL is not implementation acceptance.

1. The observed parser still compares only local names and its live test accepts
   `xmlns:md="urn:1c"` (`platform_xml.rs:19-38,156-193,222-233`). Task 4/5B must
   flip this RED before provider implementation.
2. The dirty Task 5A candidate already has `BindingRuntimeContextV1` and the
   five-field `EventHandlerModuleProfileV1`, which agrees with v4.
3. Its CFE authority is still represented as separately assemblable
   `MetadataIdentityFactV1`, `CfeObjectMembershipFactV1`, and
   `ConfigurationAuthorityFactV1` (`model.rs:2142-2265`), joined later in
   `proposal_validator.rs:638-745`. v4 requires typed source-bound whole
   companions, or an exactly equivalent non-bypassable whole-fact constructor,
   before Task 5A can be accepted.
4. Its `ScheduledJob` binding currently carries only `enabled` and runtime
   context (`model.rs:669-692,802-825`); Predefined, CommonModule capability, and
   Definition/zero-arity instance proof still require Task 5A back-propagation.
5. The Form event matrix is still physically under native operations. v4 requires
   neutral extraction and one shared import before Task 5B/Task 8 can claim
   completeness.

These are expected implementation gaps, not defects hidden by the v4 verdict.
They are explicit STOP conditions.

## 6. RED/STOP audit

PASS. Section 14 has minimal REDs for:

- base/adopted whole CFE companions and wrapper-UUID decoys;
- exact MDClasses URI and foreign namespace lookalikes;
- selected-source exact-set equality, duplicate uniqueness, every event/signature
  class, module-property witnesses, same-source runtime context, and Definition
  arity/export;
- scheduled Global/Server, Predefined true/false, zero arity, Procedure/Function,
  and ServerCall non-materiality;
- neutral Form registry parity, Button, missing kinds, BaseForm, zero/multiple
  Actions, call types, item/context and whole-document completeness;
- exact missing-Form subjects and atomic result-limit groups;
- one composite Task 7 invocation and group-local SourceSetWide isolation;
- same-byte processing determinism versus changed-byte identity;
- verified-reader OS containment.

Section 15 orders back-propagation before infrastructure and has explicit STOPs
for every former unsafe shortcut. Section 16 refuses acceptance until Task 5A,
Task 6/7/8, active spec/product contracts, full tests, clippy, Windows compile,
and diff checks agree.

## 7. Contradiction scan

No unresolved normative contradiction was found inside v4.

Resolved contradictions:

- `ServerCall=false` was removed from EventSubscription validity because it is
  absent from the exact five-field primary requirement and conflicts with an
  official supported pattern.
- `ServerCall=true` was removed from general ScheduledJob validity; it remains a
  valid example profile, not a requirement.
- declarative runtime context is no longer conflated with BSL annotations.
- a non-predefined scheduled metadata template is no longer treated as proof of
  a runtime instance.
- Form `Button` is no longer listed event-capable; zero Action is no longer
  treated as a complete command with no runtime binding.
- Task 7 no longer claims separate Metadata invocations.

The only open condition is external/operational G0 and the implementation gaps
listed in section 5. Therefore the correct verdict is conditional design PASS,
implementation STOP.

## 8. Final verdict

**PASS-CONDITIONAL.** Task 5B v4 is the authoritative design for the provider
slice and closes all known P0/P1/P2 ambiguities. Do not begin Task 5B production
implementation until an owner accepts a clean Task 5A commit that satisfies
section 3 and records its exact SHA. After that commit, rerun this audit against
the accepted code/spec and treat any mismatch as a new blocking review finding.
