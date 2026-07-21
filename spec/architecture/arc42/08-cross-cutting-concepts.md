# 8. Сквозные концепции

## Single Public MCP

The LLM sees one server and does not coordinate multiple MCP caches or indexes.
This is the primary token and context saving mechanism.

## Dry Run Safety

Mutating tools default to dry-run when they can produce an honest preview.
Skills apply them only for explicit user-requested mutations. A mutation without
an honest preview uses its declared closed-variant policy, not a universal
sandbox: `localJournaled` atomically records owned local creation or a task
decision and has no external effect; `contained` alone may mutate an owned
probe/sandbox/evidence area; `preparedJournaledEffect` requires the exact
prepared/session/status digest; and `journaledEffect` requires the exact guard
or recovery digest. Both external-effect policies write intent before effect
and verify or reconcile the postcondition. None returns a fabricated dry-run,
and sandbox prepare/apply is used only by variants that are actually
`contained`.

## Cache Ownership

The orchestrator owns cache state. Adapter calls must report through application
use cases so domain events and cache invalidation cannot be bypassed.

Durable repository-operation state is a separate class. It uses stable task and
operation identities, write-ahead remote-effect stages, advisory leases, atomic
synced records, explicit recovery, and schema migration. It is never inferred
from cache freshness.

A non-overridable per-user coordination locator binds canonical targets to the
registered durable root and unresolved tasks. `UNICA_STATE_DIR` changes cannot
create a second operational history for the same original/repository identity.

## Internal Adapter Pattern

Adapters are typed boundaries around existing engines. They may use CLI or MCP
protocol internally, but their names and cache lifecycle are not exposed to LLM.

Python/PowerShell/Bash operation files are not a runtime adapter class for
developer operations. Donor scripts can be kept only as fixture reference models
for native `unica.*` MCP handlers.

## Workspace-scoped Services

Some internal adapters may run behind hidden workspace services. These services
are owned by `unica`, scoped by workspace and source root, and coordinated
through volatile cache state. They are not public MCP registrations and must not
appear in skills as routing targets.

The lifecycle rule is lazy start, reuse while live, invalidate on domain events,
and natural exit after idle or max-age limits. Cheap read-only tools that do not
need warm analyzer/index state must not start the service.

## Source Of Truth Order

When documents disagree, use this order:

1. current code and tests;
2. package manifests and `.mcp.json`;
3. active `spec/`;
4. README and skill prose;
5. archived or research docs.

## Repository Effect Safety

Designer requests are typed and separate public, path, and secret arguments.
Known secrets are scrubbed before persistence. Repository operations succeed
only after observed postconditions; process exit and localized prose alone are
insufficient. Unknown effects fail closed and require reconciliation.

Lock compensation acts only on acquisitions attributable to the current
operation. A dedicated integration account and one original-infobase lease are
mandatory until stronger ownership discovery is proven. The account has its
own repository-plus-username persistent reservation across original infobases.

Repository history has two distinct identities. The global history cursor is
an immutable scan position; task relevance comes only from a recomputed
relevant-baseline digest and a complete contiguous semantic partition. The
configuration-root guard serializes root/support versions, not unrelated
non-root commits. Such commits may advance while the guard is held and remain
safe only when their exact partition proves the task/reference/support closure
unchanged.

Repository refresh applies only the approved selective `-Objects` set and
verifies the per-target revision/fingerprint result while root plus the exact
existing target/parent/referrer closure is locked. The platform's ignored/latest
`-v` behavior is not treated as a version-pinning primitive, and a broad latest
update cannot replace this object-set boundary. Structural confirmation may be
derived only for exact approved incoming add/delete changes with its own
capability evidence. An ordinary CF is accepted only as the registered
ordinary-result artifact used by candidate comparison/no-force sandbox merge,
never as D0/Dn. CFU is inspectable only so its typed rejection can be returned
and never becomes a branched workflow input.

Main integration binds a complete no-force support-preflight digest. A manual
editable-with-support prerequisite uses the profile-bound reserved original or
exact separate actor/working-IB mode while Unica owns no automated worker/lock,
and is offered only after an ordinary recovery distribution is verified for
every support layer reachable from the configuration-root support-settings
window and bound to a receipt-proven, profile-managed retained CF handoff
readable by the bound manual actor. A deny-unknown retention provider binds one
exact provider object, canonical source and SHA, and idempotently acquires one
task lease whose capability rejects rename/overwrite/delete through archive.
Unica may mutate only provider lease metadata, never that external content; CFU
cannot fill this role.
The authorization begins `awaitingArm`; its only instruction is for the bound
human to acquire the configuration root without editing or committing. A
separate strictly read-only `supportPrerequisiteArm` preview starts at the
authorization cursor and proves a complete all-unrelated prefix, exact current
actor root ownership, and unchanged candidate set, relevant baseline, support
graph, original fingerprint, recovery set, and retained handoffs. It accepts no
`operationId`/`dryRun`, creates no durable preview handle, and is repeated after
response loss. Only its `localJournaled` apply with `approvedArmingDigest`
changes the action to `armed` and publishes the immutable receipt/edit
instruction. A missing root
returns acquire guidance and a proven wrong owner returns release/coordination
guidance; neither arms. Exact pre-arm drift at preview or apply final recheck
keeps `awaitingArm`; neither stage arms or cancels. Both require fresh preflight
and request release iff the bound actor still holds the root; after release, a
separate fully proven cancellation must publish its own receipt. Missing
evidence remains inconclusive. The edit instruction asks the human to retain
the root, apply only the armed transitions, commit/release a separate version,
and then close Designer. Acceptance requires the exact actor/IB and armed delta,
no intervening root/support version, and the attributable version first after
the arming cursor; release/reacquire without an intervening version is
semantically admissible. Only that attributable version with a closed
manual-actor lock window is accepted, after which all Dn-and-later
evidence is rebuilt. Reserved mode closes the terminal local race under a
service-held exclusive configuration lease on the exact original; separate
mode uses the same boundary on its working IB and is not offered until that
lease has captured and released a clean repository-equal authorization
baseline. Busy/open or dirty state is an external stop: both lease and root
guard are released and proven, and Unica never resets/discards the human IB or
consumes/cancels the authorization. An unknown separate-IB lease effect after
authorization freezes the armed support-prerequisite lineage, or the distinct
no-arming `preArmSupportCancellation` lineage when it interrupted cancellation
from `awaitingArm`, rather than creating an unbound generic lease plan.

Pending actions have a typed no-authorized-action cancellation before or after
arming; arming receipt fields are present only in the latter case. Arming,
reconciliation, and cancellation partition every intervening version beginning
exactly at the authorization's expected-before cursor. Arm apply reproduces its
approved all-unrelated partition/cursor exactly and creates the receipt; armed
reconciliation/cancellation and armed frozen recovery retain that lower bound
and exact receipt, while `awaitingArm` cancellation has none. An unknown
pre-arm cancellation effect freezes without a receipt and requires a separate
observation-only recovery digest before its finalization digest may be
approved. Finalization has one immutable receipt intent for each missing guard,
update, cancellation, release, and terminal-local effect; it never repeats a
prior-operation receipt. Its stage-sensitive recheck may replan only a pre-
update gap after reverse compensation/audit; protected update-ready drift is a
capability breach. Known root/mode blockers stop with exact compensation and
unknown effects remain recovery. The workflow is unavailable without real proof
that both guards survive worker/connector death through explicit release.
Terminal status/archive retain the separate cancellation/recovery receipt pairs
and full effect/recheck/attempt/update/tail lineage. Post-release tails
cannot reopen a terminal authorization. Invalid history and
unknown lock effects fail into recovery. Frozen recovery locks only its exact
root/content/structural/reference closure and selects
`restoreThenReauthorize`, `preserveExternalAndReauthorize`, or
`restoreThenAbandon`. Proven external support remains part of the baseline;
task-attributed content/off-support taint cannot return to successful
integration. A versionless `originalNotClean` case invents no history entry and
uses only `restoreThenReauthorize`; ownership evidence may reclassify only a
persisted invalid/unattributed observation with unchanged raw deltas.
Inverse-abandonment cleanup is staged: archive `dryRun` preview returns only a
proposal/evidence digest and performs no authorization or external lease
operation. The distinct approved archive apply journals before recovery-handoff
and mode-specific lease gates, publishes `awaitingArm` only after success,
returns a typed no-authorization stop for missing/stale or proven busy/dirty
evidence, and enters recovery for an unknown lease effect.
`abandonmentReady` normally permits only status, classified
routine refresh, and abandoned archive, with narrow pending-cleanup
arming/reconciliation/cancellation and frozen status/recovery exceptions.

Raw/public force, lock stealing, and implicit merge/reference decisions are
outside automation. The sole internal exception is the capability-proven,
adapter-derived structural confirmation for the exact approved repository-
update add/delete set described above; it never applies to merge/commit/unlock.
Final task commit additionally requires a capability-proven atomic
no-force safety boundary over the immediate post-merge history/reference/
support recheck. Pre-intent relevant drift, or a post-boundary locked-target/
root change or deletion-blocking referrer, starts no accepted commit; harmless
closure expansion is retained as `nonConflictingConcurrent`. Any partial/
unproven outcome remains recoverable.

An original-fingerprint mismatch is a gate-only stale input only when an
`OriginalCleanRefreshProof` shows a clean out-of-band repository refresh with no
task merge. Unowned, local, or unclassified original state enters recovery.

## Owned Destructive Paths

External task IDs never become path components. A UUID instance, exclusive
marker/nonce, canonical containment, recursive symlink/reparse rejection,
root/home/Git/worktree exclusions, and same-filesystem quarantine guard every
cleanup. Every retention-provider root and exact recovery source is additionally
pairwise non-overlapping with owned/quarantine/original/repository/state/
coordination roots in both containment directions. Live checks repeat at start
and immediately before archive lease release; archive freezes the canonical
provider/source boundaries before exact-once release. Cleanup compares those
frozen boundaries with every quarantine/destructive target. It additionally
recanonicalizes a still-existing provider/source, but accepts external
move/deletion after verified release without inventing or touching the absent
object. An ambiguous release blocks cleanup, and provider content is never a
destructive target.
