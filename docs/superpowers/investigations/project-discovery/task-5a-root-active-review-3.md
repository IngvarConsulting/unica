# Task 5A root active review addendum 3

Immutable handoff checklist for the tracked Task 5A diff. This note is review
evidence only and is intentionally outside the tracked product diff.

## P0: closed semantic atomic groups

- Replace generic `Anchored(Vec<...>)`/union-by-material-subject grouping with the
  exact v5 closed `SemanticAtomicGroupIdV1` registry and explicit stable tags:
  StandaloneFact, source-local CfePairHalf, per-subscription
  EventSubscriptionDescriptor, per-form CompleteFormCatalog, conditional
  ScheduledJobCluster(DisabledActivation|EnabledDescriptor), per-service
  HttpServiceDescriptor, and per-owner/callback-slot
  PlatformCallbackRequirement.
- A shared runtime endpoint or shared Definition record must not merge otherwise
  independent descriptors. In particular, two EventSubscriptions that use one
  handler remain two atomic groups; Definition evidence is not part of either
  EventSubscription descriptor cluster.
- One semantic group never spans ports. Fact stable tag is only an inner-record
  ordering key, never a group discriminator.
- Add a RED/GREEN permutation test for two subscriptions sharing one handler and
  prove the groups remain independent.

## P0: canonical prefix-stop limits

- Per-port and global limit partitioning must stop retaining at the first whole
  group that does not fit. That group and every later group are dropped.
- The current skip-and-continue shape is forbidden because it can retain a later
  smaller group after an earlier oversized group and therefore is not the
  contractual canonical prefix.
- Add RED/GREEN coverage where an oversized earlier group precedes a smaller
  later group; both must be dropped and their exact material scopes gapped.

## P0: ScheduledJob activation matrix

- Plan/query Definition only for an exact supported active descriptor:
  `Use=true`, `Predefined=true`, `Global=false`, `Server=true`, valid registered
  MethodName/owner.
- `Use=false` is a complete metadata-only DisabledActivation conclusion:
  `scheduled_job_disabled`/No, independent of Predefined, MethodName, profile,
  or Definition.
- Non-predefined, unsupported profile, and missing/malformed required material
  are metadata-only Unknown and do not plan Definition for the job.
- DisabledActivation and EnabledDescriptor are mutually exclusive atomic
  branches; every physical record belongs to exactly one branch.

## P1: primary-source-bounded signature variants

- EventSubscription and ScheduledJob positive rows require synchronous
  ModuleDefault definitions.
- Otherwise-matching explicit execution contexts and async definitions are
  unsupported variants/Unknown, not signature mismatch/No.
- Hard proven mismatches (kind/export/arity as specified by the accepted v5
  matrix) remain No.

## Required closure evidence

- Focused RED/GREEN tests for every row above.
- Full locked crate tests, product contract tests, fmt check, all-target clippy
  with warnings denied, and diff check.
- Task 5A report must name this note and its SHA-256 along with the two earlier
  immutable review notes.
