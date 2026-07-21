# 10. Требования к качеству

## Token Efficiency

LLM-visible tool surface must avoid requiring the model to know which internal
engine owns which cache. Coordination belongs to `unica`.

## Safety

Mutating operations with an honest preview default to dry-run and return cache
impact before applied execution. A no-preview mutation follows its exact policy:
`localJournaled` records owned local creation or an atomic task decision with no
external effect; `contained` is limited to owned probe/sandbox/evidence effects;
`preparedJournaledEffect` requires an exact prepared/session/status digest; and
`journaledEffect` requires an exact guard or recovery digest. Both external-
effect policies write intent before effect and verify or reconcile the
postcondition. No policy fabricates a dry-run, and sandbox preparation is used
only where the variant is genuinely `contained`.

Repository-cycle mutations additionally require stable operation IDs, durable
intent/effect/postcondition records, exact ownership, capability proof, and
fail-closed recovery. The original repository binding, no-XML boundary,
no-force merge/commit/unlock rule, CFU reject-only boundary, exact selective
`-Objects` update with per-target proof, and owned cleanup policy are
non-negotiable. Neither a global cursor advance nor the platform's ineffective
`-v` option is accepted as task-relevance/version-pinning evidence.
Provider-backed recovery CFs require exact object/path/SHA binding, a
capability-proven task retention lease with rename/overwrite/delete denial, and
a canonical source below its provider root with that root disjoint from every
owned/protected root. Provider content is never an Unica mutation or cleanup
target.

## Maintainability

Domain, application, infrastructure, and MCP transport code remain separated so
adapter replacement does not rewrite public MCP handling.

## Observability

Operation result must include summary, warnings, errors, artifacts, and cache
impact. Future native adapters should add structured diagnostics without
changing the top-level result shape casually.

Every task-bound branched result requires `resultKind`, task/status,
evidence/data, and the ordinary result fields; `operationId` is required only
for mutation or when a read reports an existing operation. The strictly
read-only `supportPrerequisiteArm` preview therefore has no operation ID,
`dryRun`, or durable preview handle; its `localJournaled` apply requires the
operation ID and `approvedArmingDigest`. The closed
`completed`/`stopped`/`rejected` union requires `stopCode` only for an
evidence-bearing domain stop and forbids it for safety rejection. Raw
argv/stdout/stderr are omitted. Evidence is bounded, redacted, hash-addressed,
and sufficient to distinguish success, block, unknown effect, safe abandonment,
and completed cleanup.
Read-only creates no operation/lease/start-attempt/receipt/durable evidence or
preview handle/task-status mutation; any referenced operation already exists as
a mutating record. JSON-derived digests use RFC 8785 JCS and operation input is
domain-separated by exact tool and policy. The only durable operation states
are `registered`, `intentWritten`, `effectUnknown`, and `terminal`; observed is
an fsynced barrier within `effectUnknown`.

## Recoverability

MCP disconnect, worker death, platform failure, and repository disconnect must
be testable at every dangerous journal boundary. Recovery never repeats an
unknown external effect and never treats an unavailable owner/lock check as
proof of release. Support recovery preserves the complete immutable history,
locks only exact correction/finalization targets, and has three closed
dispositions; external support cannot be silently inverted. Manual support is
two-stage: the first instruction permits only root acquisition, and a separate
strictly read-only `supportPrerequisiteArm` preview must prove the
authorization-anchored all-unrelated prefix, unchanged candidate/relevant
baseline/support/original/handoffs, and the bound actor's current root
ownership. It is repeatable after response loss and persists no handle; only its
`localJournaled` apply with `approvedArmingDigest` may publish the immutable
receipt that permits editing. Missing root returns acquire guidance and proven wrong owner
returns release/coordination guidance without arming. Pre-arm exact drift
never arms: preview-stale and apply-final-recheck-stale both keep
`awaitingArm`, and neither cancels. Both require fresh preflight and request
release iff the bound actor still holds the root; a separate cancellation with
complete proof/receipt follows only after release. Reconciliation accepts only
an `armed` action with exact actor/IB and armed delta, no intervening
root/support version, and whose version is the first after the arming cursor.
The instruction asks the human to retain the root, but release/reacquire without
an intervening root/support version is semantically admissible. Separate-IB
baseline capture and terminalization prove an exclusive service lease, while
reserved-original terminalization proves the corresponding original-IB lease;
both require a closed Designer session and return typed busy/dirty stops without
resetting the human IB. All prerequisite/cancellation/frozen partitions retain
the authorization cursor as their lower bound. A post-authorization unknown
separate-IB lease effect stays in armed support-prerequisite recovery or, for
an awaiting-action cancellation, the no-arming pre-arm cancellation recovery; a
real capability fixture must prove its root/mode guards survive worker/connector
death until explicit receipt-proven release. Its separately approved finalizer
maps one intent/receipt to each missing effect, rechecks stage-sensitively,
compensates/audits only pre-update replans, treats update-ready protected drift
as a breach, and retains distinct cancellation/recovery receipts in terminal
status/archive. A
versionless dirty-original recovery invents no repository version, ownership
reclassification cannot rewrite positive evidence, and post-terminal tails
cannot reopen the old authorization. Retention acquire/replay/release is
idempotent; archive precedes release and an ambiguous release blocks cleanup.
Cross-host recoverability additionally requires durable fenced reservation
receipts: local files/mutexes are not global exclusion, and a multi-host
endpoint requires retained coordinator proof before start.

## Fidelity

Full-cycle completion includes modify, add, delete, form/attribute ownership,
references, multi-vendor support, complete target-support preflight, safe
acquire-root -> repeatable read-only arm preview -> `localJournaled` apply ->
instructed root retention and exact actor/IB/delta/version-order commit/release
-> Designer closure ->
reconciliation, or no-action root release plus Designer closure -> cancellation,
in both target modes,
manual-actor lock closure, inverse abandonment cleanup, cursor/partition-based
relevant concurrency, `staleSupportPreflight`, conditional
`abandonmentReady`, atomic commit safety, one final task-content version,
recovery, archive, and cleanup. Narrower milestones cannot redefine this scope.

## Packaging Reliability

Generated packages must verify checksums and must not depend on globally
installed tools when bundled equivalents exist.
