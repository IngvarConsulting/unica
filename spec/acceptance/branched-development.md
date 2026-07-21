# Branched Development Acceptance

## Goal

Prove that an installed Unica package can complete a development task against a
main 1C configuration repository while the original infobase remains bound,
all development happens in an owned disposable branch, integration is scoped
and recoverable, and cleanup occurs only after a proven terminal outcome.

This matrix is normative for [issue #137](https://github.com/IngvarConsulting/unica/issues/137).
A milestone, mock-only implementation, or happy-path demonstration cannot close
the issue while any row lacks the stated evidence.

Exact request/response schemas, execution policies, lock mapping, and stable
errors are normative in the
[Branched Development Tool Contract](../contracts/branched-development-tools.md).

## Requirement Matrix

| ID | Requirement | Required evidence |
| --- | --- | --- |
| BD-01 | Original infobase remains bound to the same repository; XML is never loaded into it | Real fixture records binding identity before/after every phase and a Designer invocation transcript with no original-target XML load |
| BD-02 | D0 and every refresh Dn are verified full distributions; result CF is ordinary; out-of-scope CFU is recognized only to be rejected | Artifact probe tests plus real create/deploy fixture and strict tool schemas |
| BD-03 | Each task gets a new owned File IB/XML workspace and cannot reuse a stale baseline | Task-state/path unit tests, packaged second-cycle E2E, marker and instance-ID evidence |
| BD-04 | Only actually changed objects become editable in the exact support layer; unchanged objects relock, new vendor objects are non-editable, all other modes and upstream chains survive | Layer-aware parser round-trip tests, target support-preflight evidence, and real unsupported/multi-vendor fixtures |
| BD-05 | Synchronization is a true L1+D0+D1 supported update before repository locking | Invocation/evidence order and real twice-changed property fixtures |
| BD-06 | Pre/post canonical project delta is `equivalent` or explicitly `adapted`; missing/extra changes stop | Pure delta tests plus fake and real supported-update scenarios |
| BD-07 | Every scalar/module/add-add/delete-modify/reference/support conflict is typed and explicitly resolved | Settings/parser fixtures, decision journal, sandbox replay and real platform cases |
| BD-08 | Added objects preserve UUIDs; lock plans cover modify/add/delete/forms/attributes/references with a reason per object | Ownership/reference graph unit tests and real integration scenarios |
| BD-09 | A foreign lock stops integration within finite per-call and whole-transaction deadlines; no polling occurs, every lock acquired by that operation is released and verified without touching pre-existing locks, and the failed target may be the configuration root or a development object | Two-user real fixture, first-root-conflict and development-object conflict fixtures, hung-call/transaction-timeout/partial-effect fakes, durable journal and final lock proof |
| BD-10 | Relevant repository changes force a new distribution/rebase; unrelated commits do not. Every repository refresh binds a complete contiguous history partition and uses only the exact selective `-Objects` set under root-first target/parent/referrer locks; no version-pinned `-v` selector is assumed. Exact incoming add/delete additionally uses only the capability-proven structural confirmation | Relevant-object/history-partition fixtures, root/target lock ordering and compensation, exact selective-update digest/fingerprints, and concurrent histories including add/delete and unrelated commits |
| BD-11 | Main merge touches only approved delta/reference closure and preserves target support state | Pre/post original fingerprints and unsupported/upstream-supported real targets |
| BD-12 | Maximum configuration, UUID, reference, delta, diagnostics, and lock checks pass before commit | Immutable validation receipts produced while the exact owned lock set is held |
| BD-13 | Exact verified task-content integration set, including add/delete entries without their own lock, is committed once with the frozen task-bound comment in one final repository version and all acquired task/guard locks are released. The post-merge history partition is endpoint-bound, and a capability-proven no-force atomic commit safety boundary rejects a concurrent locked-target/root change or new referrer that blocks an approved deletion before task-content effect. A harmless capability-proven closure expansion may be retained as `nonConflictingConcurrent`; it is not mislabeled as unrelated. Normal support prerequisite/surplus-cleanup versions are root-only; exceptional recovery versions may only restore explicitly observed invalid content. Ambiguous/partial outcomes never retry blindly | Real success, unrelated-concurrent success, harmless non-conflicting closure expansion, concurrent deletion-blocking referrer atomic rejection with zero partial commit, support-guard release, invalid-content restoration, missing-lock, broken-reference, comment-policy, interruption, and disconnect fixtures |
| BD-14 | MCP/worker/process interruption resumes from durable state at every remote-effect barrier; `observed` is an fsynced barrier, never an operation state, and a lost terminal response reconstructs typed gate-history, separate-IB lease, support-recovery finalization, and post-merge commit evidence from status/receipts and a digest-bound exact recovery plan. A state-root override cannot hide an unresolved task/start attempt, and persistent target/account reservations survive lease loss and response loss | Unit crash matrix with no `state: observed`, response-loss-after-observed-barrier tests, two-host/two-account reservation contention, recovery-plan/receipt approval tests, cross-root locator tests, fake worker death, and real dangerous-phase restart scenarios |
| BD-15 | Successful archive is complete and redacted; disposable data is deleted only after commit/unlock/archive proof and marker identity matches project/original/repository journal identities | Archive schema tests, known-secret scan, owned-path/identity/quarantine tests and E2E |
| BD-16 | Abandoned tasks can be archived/cleaned only after original equality, no locks/worker/unknown effect or pending/frozen manual action, and exact inverse reconciliation of every task-only support transition. A cleanup `dryRun` preview returns only a proposal/evidence digest and performs no external lease operation; only a distinct approved, pre-effect-journaled archive apply may publish `awaitingArm` after its gates pass | State transition, pending-action cancellation barrier, proposal-versus-apply authorization staging, no-lease preview, missing/known-busy-or-dirty typed stops, unknown-lease-effect recovery, inverse reconciliation, and unsafe-abandon rejection tests |
| BD-17 | Public surface is one server `unica`, 21 strict lifecycle tools, required layer-aware `unica.support.edit`, capability-gated general developer mutations, and MCP-only `unica:branched-development`. No exact BSL writer is a global release prerequisite; when one is advertised, it must support branched task/resolution contexts and durable receipts. Its absence blocks only scenarios that actually require BSL text mutation | Registry/schema tests, conditional companion-tool contracts, with-writer and without-writer skill transcripts, source and generated-package smoke |
| BD-18 | A release claims OS/platform/locale/original-IB/repository-transport behavior only through an exact platform-capability row and retention-provider behavior only through a separate exact retention-provider-capability row. A manual handoff binds both IDs: the retention row proves held-object/storage guarantees, while a distinct platform row/case proves Designer readability of the same ordinary CF/SHA for the bound actor | Both versioned manifests are validated independently against hashed stored acceptance evidence; cross-manifest ID resolution, missing/stale/mismatched rows, skipped cases, and digest/evidence mismatch are negative tests |
| BD-19 | Credentials and known secret values never enter public result, journal, logs, archive, or MCP stdout | Typed-secret argv tests and byte scan of all retained fixture artifacts |
| BD-20 | Unknown metadata/support/platform behavior fails closed instead of using name-only or broad-lock approximations | Negative contract tests returning stable unsupported/capability errors |
| BD-21 | Full feature/data-migration tests finish in synchronized task IB before locks; the original gets no automatic `UpdateDBCfg`, DB restructuring, or destructive runtime test | Invocation-order assertion plus task validation receipts and original Designer transcript |
| BD-22 | Every tool obeys the exact generated request/result schema, per-variant mutability/preview policy, closed completed/stopped/rejected union, typed data/evidence, and operation replay rules. JSON-derived digests use RFC 8785 JCS only, and operation inputs are domain-separated by exact tool name and execution policy | Committed generated JSON Schema/variant-policy snapshots plus application and packaged MCP contract tests, including evidence-preserving domain stops, data-free safety rejections, canonical-JSON golden vectors, and different-tool/different-policy replay negatives |
| BD-23 | Every stable error code maps one trigger to the required phase/stop/recovery/cleanup behavior | Table-driven domain/application tests for the complete tool-contract error table |
| BD-24 | File and client/server originals plus file/server repository transports are enabled only by their own topology evidence. Each endpoint declares `hostConfined` or `multiHost`; any multi-host topology requires retained proof of a reachable linearizable coordinator atomically excluding target and repository-account starts | Separate disposable real-fixture rows; same-target and shared-account two-host races, response-loss receipt observation, cross-topology and missing-coordinator negative preflight tests |
| BD-25 | Every typed development capability advertised and selected for the owned branch, plus the required build and test tools, operates through `branchedTask` ID resolution, durable mutation receipts where mutation occurs, and task-context cache events without exposing a task path in any result field | Application-port/schema tests plus packaged transcripts covering every selected development mutation, build, and test call; a BSL call is conditional on a compatible advertised writer, and the without-writer case proves a capability-gated skill stop before mutation with no fallback; recursive structured/free-text path-projection scans cover both cases |
| BD-26 | Main integration derives a complete UUID/ownership/reference candidate set, runs the real disposable merge without force, and returns exactly `ready`, `manualSupportRequired`, `vendorForbidsChanges`, or `supportPreflightInconclusive`; XML paths are never authoritative and CFU is never accepted. A pre-existing off-support candidate is preserved without a manual transition, while any newly required detachment is forbidden | Pure candidate-closure tests, localized/incomplete diagnostic fakes, schema snapshots, real ready/manual/forbidden/inconclusive support fixtures, and existing-off-support preservation versus new-detachment cases |
| BD-27 | A manual support action is one digest-bound root-only version in profile-bound `reservedOriginal` or `separateWorkingInfobase` mode and is offered only with a verified ordinary recovery distribution and a task-leased, receipt-proven, profile-managed user-visible CF handoff for every support layer reachable from the configuration-root support-settings window; CFU cannot satisfy either. The human must first acquire only the configuration root without editing or committing. The exact `supportPrerequisiteArm` preview is strictly read-only: it requires `taskId`, accepts no `operationId` or `dryRun`, creates no durable preview handle, and is simply repeated after response loss. Its separate `localJournaled` apply requires `operationId` plus `approvedArmingDigest`, repeats the authorization-anchored all-unrelated-prefix and unchanged candidate/relevant-baseline/support/original/handoff checks under the bound actor's current root ownership, and only then publishes the immutable arming receipt/edit instruction. The instruction asks the actor to retain the root through commit, but acceptance is based on the first root/support version after the arming cursor, exact actor/IB, exact armed delta, and absence of an intervening root/support version; release/reacquire with no such intervening version is semantically admissible. Pre-arm drift permits no edit and requires fresh preflight: both exact `SupportArmStaleData` variants, `stage=preview` and `stage=applyRecheck`, leave `awaitingArm` unchanged, neither arms nor cancels, and after any held root is released a separate `supportPrerequisiteCancellation` must produce its complete proof and cancellation receipt. The deny-unknown retention provider binds one exact object/path/SHA, proves actor readability plus rename/overwrite/delete denial through archive, and is never a content-mutation or cleanup target. Its `retentionCapabilityRowId` resolves only in the tracked retention-provider manifest; the distinct `manualReadabilityCapabilityRowId` resolves only to the exact platform row/case proving Designer readability of the bound CF/SHA. Reserved mode holds a capability-proven exclusive original-IB lease through terminalization as well as restoring the actor's empty lock baseline; separate mode captures a clean authorization baseline and holds the corresponding working-IB lease. Arming, pending reconciliation/cancellation, and frozen recovery retain a contiguous history partition anchored at the authorization cursor. Armed frozen recovery selects exactly `restoreThenReauthorize`, `preserveExternalAndReauthorize`, or `restoreThenAbandon`, preserves routine plus proven external-support history, and terminalizes only under the persisted root/target finalization guard; interrupted pre-arm cancellation instead has no receipt/disposition and uses observation then separately approved exact finalization. Accepted/corrective versions and arming receipts are archived, every retention lease is released exactly once only after durable archive lineage, and fresh relevant evidence requires Dn/rebase/preflight | Both-mode real fixtures; acquire-root/arm/edit/commit ordering; arm preview has `taskId` but no `operationId`/`dryRun`/durable handle and is repeatable after response loss; apply is `localJournaled` and digest-bound; missing/wrong root, both exact stale stages (`stage=preview`/`stage=applyRecheck`) preserving `awaitingArm` followed by separate fully proven cancellation, inconclusive evidence, apply replay/response loss, release/reacquire without an intervening root/support version, and first-root/support-version/exact-actor/IB/delta checks; every root-reachable layer recovery distribution and provider-backed handoff present/readable/missing/stale/CFU cases; separate retention/platform manifest resolution, exact-case/evidence validation, provider exact-object mapping, lease acquire/replay/held-observation/release, overwrite/delete denial, nested-root/path rejection, and ambiguous-release cleanup block; real Designer restoration from the referenced CF; reserved-original and separate-IB clean/busy/dirty/unknown lease cases; authorization-cursor partition anchors; root/target ordering and compensation; versionless `originalNotClean`; all three dispositions, external-overlap coordination, immutable-positive versus prior-unattributed ownership reclassification, post-terminal tails, response loss, selective original refresh, fresh-Dn proof, and packaged resume transcript |
| BD-28 | A current `ready` support gate carries endpoint-bound `SupportGateHistoryEvidence` and is rechecked after acquiring the root guard first. Only a fully classified unrelated cursor extension may reuse it; relevant history requires fresh Dn, while a non-anchor mismatch enters/clears `staleSupportPreflight` through exact unlock. Original-fingerprint staleness additionally requires clean-refresh/no-task-merge proof; unowned deltas recover. Successful original merge consumes the gate into receipt/post-fingerprint history lineage, and commit uses endpoint-bound post-merge evidence plus the capability-proven atomic safety boundary | History endpoint/digest and net-relevant-tail tests, clean-versus-unowned original drift, root-first stale compensation/unlock exits, original-apply boundary crashes, consumed-gate response loss, unrelated concurrent commit, deletion-blocking-referrer zero-effect rejection, harmless closure expansion as `nonConflictingConcurrent`, and schema/status snapshots |

## Public Tool Matrix

This table is a completion summary; the exact schemas and data variants in the
tool contract are required, not optional implementation detail.

| Tool | Mutation boundary | Required guard or postcondition |
| --- | --- | --- |
| `unica.branched.start` | Durable journal and owned work root | Valid deny-unknown profile, task ID, operation ID, exact platform row plus separate retention-provider row, and authoritative pre-creation atomic target/account reservations. Local mutexes protect only local processes; an endpoint not capability-proven `hostConfined` requires the row's linearizable shared coordinator/cross-host exclusion evidence. Exact per-layer immutable CF source/provider-object mapping and WORM-retention entries, pairwise-disjoint protected/provider roots, and both-mode conditional inspection/exclusive-configuration-lease endpoint capability remain required; reject missing/stale/mismatched rows, skipped cases, digest/evidence mismatch, or unproven coordination before task creation |
| `unica.branched.status` | None | Strictly read-only; first call after reconnect; returns current typed IDs/digests/previews needed for the next legal call without paths |
| `unica.branched.archive` | Durable archive | Verified success or safe abandonment; no pending/frozen support action or task-only transition. For inverse cleanup, `dryRun` returns only the proposal/evidence digest with no authorization or external lease acquire/inspect/release; the distinct approved apply journals before lease gates and publishes `awaitingArm` only after they pass. Missing/stale or known busy/dirty evidence is a typed stop; unknown lease effect is recovery. Durable handoff lineage precedes exact-once retention-lease release and an ambiguous release remains recovery-bound |
| `unica.branched.cleanup` | Owned disposable root | Archived outcome, marker/nonce/containment/reparse checks, quarantine first, and frozen canonical provider/source boundaries checked against every destructive target. After verified archive-time lease release, a still-live provider/source is recanonicalized and revalidated; its external move/deletion is admissible and cleanup never invents or touches it |
| `unica.delivery.inspect` | None | Exact name/vendor/version identity, distribution/update permissions, warnings-as-errors, binding, repository/main/database equality, rules, support layers, stable digest |
| `unica.delivery.create` | Distribution artifact | Target-effect-free preview has no output ID/hash/time. `refreshDistribution` is legal only from the exact six specified phases and after current local-checkpoint/clean-inspection/capability and no-worker/no-lock/no-original-difference/no-unknown-effect gates; `blockedByForeignLock` additionally needs that conflict's verified compensation and empty owned-lock set. Apply rechecks, invalidates every Dn-and-later descendant proof, and returns `localVerified` |
| `unica.delivery.verify` | Probe IB only | Prove artifact/support behavior; never trust extension; destroy probe only after durable observation |
| `unica.delivery.deploy` | Task File IB and workspace | Preview has no allocated IDs/post-effect fingerprints; apply uses verified distribution, owned destination, current=vendor proof and baseline fingerprints |
| `unica.merge.compare` | Evidence only | Typed allowed sides, UUID/property manifest plus platform report; reject name-only mapping |
| `unica.merge.prepare` | Variant policy: `supportedUpdate` plus replacement/resolved replay are `contained`; `mainIntegration` is `journaledEffect` plus contained disposable sandbox work | Immutable checkpoint/anchors and operation ID. `mainIntegration` journals intent before the first retention-provider or mode-specific working-IB lease effect; it publishes authorization only after exact lease/postcondition observation, sends an unknown external lease effect to typed recovery, and still performs the no-force merge only in the contained sandbox |
| `unica.merge.conflicts` | None | All seven typed conflict kinds, three-side hashes, and per-kind allowed explicit resolutions |
| `unica.merge.resolve` | Decision journal | Typed payload, rationale, owned/fingerprinted manual data |
| `unica.merge.apply` | Named task/original target | Prepared/resolved session, intent-before-effect journal, fresh anchors, replay equality; task apply atomically publishes a validated full staged dump and proves IB/XML equality, while original also binds integration/lock sets and atomically consumes the ready gate into receipt/post-fingerprint lineage |
| `unica.merge.verify` | Evidence/checkpoints | Local scope creates the first immutable checkpoint; equivalent/adapted synchronized-task scope creates the required post-update checkpoint; pre-lock main-sandbox uses the current gate and post-merge main-integration uses only its exact consumed lineage |
| `unica.repository.status` | None | Binding, main/repository/database sync, journal/last-observed conflicts with completeness, nullable unproven owner fields; no global-live-lock or reconciliation claim |
| `unica.repository.update` | Original configuration or local support authorization | Ordinary effecting variants use target-effect-free exact preview/digest. `supportPrerequisiteArm` is the explicit exception: its preview is strictly `readOnly`, requires `taskId`, accepts neither `operationId` nor `dryRun`, persists no durable preview handle, and is repeated after response loss; its separate apply is `localJournaled`, requires `operationId` plus `approvedArmingDigest`, repeats the full observation, and alone publishes the immutable arming receipt/edit instruction. A missing root returns acquire guidance and a wrong owner returns release/coordination guidance without arming. Exact pre-arm drift at either preview or apply-final-recheck leaves `awaitingArm` unchanged: neither stage arms or cancels; both give release guidance iff the bound actor still holds the root, require fresh preflight, and are followed only after release by a separate fully proven `supportPrerequisiteCancellation`. Missing evidence remains inconclusive. The human instruction asks that the root remain held, but reconciliation accepts by evidence: exact actor/IB and armed delta, no intervening root/support version, and the attributable version first after the arming cursor; release/reacquire without an intervening root/support version is not itself a rejection. Prerequisite/cancellation apply preserves the same authorization anchor with no gap through its guarded/post-release ranges. Root-first exact target closure plus selective `-Objects` never uses a version-pinned selector; structural confirmation is limited to approved incoming add/delete. Reserved mode holds the exclusive original-IB lease and separate mode the exclusive working-IB lease through terminalization |
| `unica.repository.planLocks` | Evidence only | Current `ready` support-gate and endpoint-bound history-evidence digests, exact integration add/modify/delete set, independently acquired development-object/reference closure, and mandatory root `supportGraphGuard` that freezes support presence or absence |
| `unica.repository.lock` | Repository locks | Approved exact plan, root guard first plus under-lock gate recheck, then per-target journal under the dedicated user and compensation proof; conflict returns `failedTarget: RepositoryTargetIdentity`, presentation-only display, nullable owner, redacted diagnostic, and external action |
| `unica.repository.unlock` | Task-owned locks/config | No force; exact proven subset only |
| `unica.repository.commit` | Repository content/locks | Exact task integration/acquired lock sets and consumed gate/merge/post-fingerprint lineage plus endpoint-bound post-merge partition remeasured at preview and immediately pre-effect; capability-proven atomic no-force rejection of concurrent locked-target/root change or a referrer that blocks approved deletion, with harmless closure expansion retained as `nonConflictingConcurrent`; frozen task comment, one call, content/unlock proof; prior support prerequisites remain separate audit evidence |
| `unica.repository.recover` | Journaled recovery target | No new merge decision; frozen support plans remain anchored at the authorization cursor, preserve complete routine/external history, one of three dispositions, the mode-specific IB lease proof, and root-first exact finalization guard/receipt. Unknown separate-IB lease effects after authorization freeze armed `supportPrerequisite` lineage or awaiting-cancellation `preArmSupportCancellation` lineage; known blockers remain typed stops and ambiguity remains `recoveryRequired` |

All effecting mutating requests require `taskId` and caller-stable
`operationId`. The strictly read-only `supportPrerequisiteArm` preview is the
explicit exception: it accepts neither `operationId` nor `dryRun` and creates no
durable preview handle; only its `localJournaled` apply requires the operation
ID and `approvedArmingDigest`. Constant
safety behavior is not represented by user-selectable booleans. Every schema
rejects raw argv, executable paths, credentials, CFU, and raw/undocumented force
controls.

Contract tests additionally prove that distribution create cannot route through
ordinary `make`, merge apply rejects an unprepared/unresolved session,
repository lock rejects an unapproved plan, and cleanup rejects incomplete
terminal/archive proof.

Schema and skill transcripts distinguish all result variants. In particular,
twice-changed properties, support-preflight stops, unexpected delta, and a foreign lock return
`ok: false`, `resultKind: "stopped"`, their stable `stopCode`, typed evidence,
and resumable handles. Safety/precondition rejections return
`resultKind: "rejected"` and `TaskErrorData` only; they cannot masquerade as a
domain stop or reuse its evidence payload.

Stable-code tests prove one semantic trigger per code. In particular,
`manualSupportRootLockRequired` proves that a missing root returns the exact
acquire-root instruction, while a root proven to another actor returns typed
release/coordination guidance; neither publishes an arming receipt/edit
instruction or changes `awaitingArm`. `supportPrerequisiteArmStale` proves exact
pre-arm history/support/original/handoff drift has two stages: read-only
`stage=preview` and apply-final-recheck `stage=applyRecheck` both leave
`awaitingArm` unchanged; neither arms nor cancels. Both require fresh preflight
and root-release guidance iff the bound manual actor is still proven to hold it;
only after release may the separate `supportPrerequisiteCancellation` produce
the full proof and durable cancellation receipt. A dedicated both-mode fixture
commits the exact proposed root/support transition before arming. It must never
retro-create an arming receipt or enter normal reconciliation/frozen
receipt-required recovery: the stale action remains `awaitingArm`, cancellation
classifies the complete version as `preArmExternal`, preserves it as an external
baseline, cancels with arming fields absent, and routes to the relevant-advance
phase. Extra-content/off-support variants are likewise preserved for fresh
preflight rather than accepted as task work; missing attribution/coverage stays
typed cancellation-inconclusive until evidence is complete. Incomplete classification remains
`supportPreflightInconclusive` without cancellation or arming;
`manualSupportLocalChangesRemain` is the single mode-specific manual-IB closure
failure and its closed data union distinguishes known lease-busy from
lease-acquired-dirty; `supportCorrectionPending` and
`supportConflictResolutionPending` retain their different corrective versus
external-ownership waits; a valid correction that changes/materializes the
frozen plan yields `supportRecoveryReapprovalRequired` and no finalization
effect until the fresh digest is approved; `supportRecoveryBlockedByLock`
requires a known failed finalization target plus verified compensation. An unknown guard/lease/update
effect uses recovery-required effect codes instead of any retryable stop.
For an interrupted cancellation from `awaitingArm`, crash fixtures at every
root-guard acquire/release, mode-lease acquire/release, selective original
update, and authorization-terminalization barrier must instead expose
`target=preArmSupportCancellation`, a frozen authorization with
`freezeKind=preArmCancellationEffect`, and no arming receipt or support-recovery
disposition. Its first approved recovery is observation-only. A complete
observation returns `recoveryReapprovalRequired` with a different digest; only
the separately approved finalize plan may run the closed action order
root-acquire, mode-acquire, stage-sensitive recheck, optional selective update,
authorization cancellation, mode release, root release, and terminal local
recovery receipt. Every effect has one immutable intent/receipt mapping;
already-observed effects are not repeated. Pre-update recheck drift may publish
a fresh plan only after exact reverse compensation and an appended attempt
audit. Drift protected by an already-held/update-ready guard is a capability
breach, never a no-effect replan. A known foreign root or busy/dirty mode lease
returns `preArmCancellationRecoveryBlocked` with its empty/compensated prefix;
the fresh current recovery plan durably carries the closed blocker evidence and
exact external instruction, so status reconstructs both after a lost stop
response. Unknown acquisition/release keeps recovery current. The platform fixture must
prove both guards survive worker/connector death until explicit receipt-proven
release, otherwise the workflow is disabled before authorization. Unknown
observation keeps the old plan current.
Both-mode response-loss tests prove the terminal receipt/status/archive retain
the pre-arm effect observation, full immutable finalization plan with every
prior compensated-attempt audit, completed progress with full effect receipts,
receipt-plan/recheck/attempt lineage, distinct
cancellation and pre-arm recovery receipt pairs, full selective-update proof,
post-release partition, and never turn this path into armed recovery.
Separate-IB recovery tests additionally prove that pending correction carries a
desired closure without a future cursor/version map and that observed history
materializes both closure and finalization plans before reapproval.

Manual-action history tests start every arming, prerequisite, and cancellation
preview at `authorization.expectedBeforeHistoryCursor`, enumerate every
repository version through the preview boundary. Arming apply must reproduce
its approved partition and cursor byte-for-byte, with no later suffix, and its
partition contains only unrelated routine history through that arming cursor.
Prerequisite/cancellation apply reproduces its approved range plus only the
contracted contiguous under-guard suffix. The arming receipt freezes the exact
prefix. Reconciliation is legal only from `armed` and
accepts the authorized version only when it is the first root/support version
after that cursor. The frozen recovery partition keeps the same authorization
anchor and arming receipt. A fixture that drops the first intervening version,
moves `fromExclusive`, introduces a gap, inserts a root/support version before
the authorized version, or substitutes an arming receipt cannot make that
arm/reconcile/cancel apply directly arm, consume, or cancel the authorization.
Post-arm invalid history freezes; only its exact recovery may later terminalize
the action. Versions observed after durable
terminalization remain a post-release tail used only to select the safe
resulting phase or a new gate/recovery; they can neither reopen nor mutate the
terminal old authorization.

`DeferredRepositoryAdvance` acceptance covers its three closed variants.
`classified` retains the capability-proven immediate successor, its
classification/semantic delta, and `observationDigest`; `unclassified` retains
the proven immediate successor plus the non-empty semantic evidence gaps; and
`coverageUnknown` retains only `fromCursor`, the coverage gap, and digest, with
no invented `firstObservedVersion`. A routine preview with incomplete coverage
or classification returns the typed deferred-advance inconclusive stop and
preserves both the handle and current phase. A complete routine preview also
preserves the handle while binding the exact resolution to its
`observationDigest`. Only a verified approved routine apply that reproduces
that digest and complete partition may atomically consume the handle and route
to the bound safe relevant-advance phase; no other authoritative call or
preview may consume, skip, or fabricate it.

Manual-action recovery tests include the versionless `originalNotClean` case:
no repository version observation is invented, the exact expected/observed
original fingerprints are retained, and only `restoreThenReauthorize` may
selectively restore the classified repository baseline and cancel the action.
They also prove that `ownershipReclassified` accepts only a previously persisted
`invalid/unattributed` observation with the same repository version and raw
root/content deltas. An authorized, action-owned, external-owned, routine,
corrective, or any other positive observation is immutable and can never be
reclassified to escape its disposition.

Both manual-target fixtures first instruct the human to lock only the
configuration root without editing or committing. The strictly read-only
`supportPrerequisiteArm` preview has no `operationId`, `dryRun`, or durable
handle and is repeated after response loss. Its `localJournaled` apply requires
`approvedArmingDigest`, then proves the exact bound actor currently owns that
root and that the authorization-anchored prefix, support graph, original
fingerprint, recovery set, and retained handoffs are unchanged. Only the
completed apply publishes the edit instruction, which asks the human to retain
the root, perform only the armed transitions, commit/release a separate version,
close the relevant Designer session, and resume through status. Reconciliation
does not claim continuous-lock evidence: it requires the exact actor/IB and
armed delta, no intervening root/support version, and the attributable version
first after the arming cursor; release/reacquire without such an intervening
version is accepted. Under the
repository root guard used for reconciliation, reserved mode acquires an
exclusive configuration lease on the exact original and separate mode on the
exact working IB; the lease is held through durable consume, cancel, or frozen-
recovery terminalization. Busy/open, dirty, acquire-unknown, and release-unknown
cases perform no unproven terminalization. In particular, an unknown separate-
IB lease effect after an authorization exists freezes armed
`supportPrerequisite` lineage or, when awaiting-action cancellation was
interrupted, no-arming `preArmSupportCancellation` lineage; neither may
degrade into a generic `manualWorkingInfobaseLease` recovery that loses the
action binding.

Retention-provider tests distinguish profile validation, tracked capability
evidence, and runtime evidence. `retentionCapabilityRowId` resolves only through
the closed tracked
`plugins/unica/references/branched-development/retention-provider-capabilities.json`;
the distinct
`manualReadabilityCapabilityRowId` resolves only through the platform manifest
and its exact Designer-handoff-readability case for the same ordinary CF/SHA.
Cross-manifest IDs, unknown kinds/providers, missing/stale rows or secret
references, provider/host/storage kind or version mismatch, skipped/duplicate
cases, evidence/contract/harness digest mismatch, mismatched provider
object/source/SHA, a source outside its provider root, a provider root
overlapping an owned/protected root, symlinks/reparse points, and initially
unreadable sources reject start. The retention row's closed canonical case set
proves acquire/replay idempotency, exact held-state observation, manual-actor
readability of the bound SHA, held rename/overwrite/delete denial, exact-once
release/replay, unknown-effect reconciliation, and canonical path/traversal/
symlink/reparse containment. A provider/lease/readability loss after start makes
support preflight inconclusive and publishes no authorization; an unknown
acquire effect remains typed recovery evidence rather than guessed success. A
successful task-scoped acquire replay returns the same lease and receipt rather
than acquiring a second lease. The same object/SHA/readability is revalidated
before a corrective instruction. Archive first persists the complete handoff
lineage and frozen canonical provider/source boundaries in an
`ArchiveStagingReceipt` whose file and parent-directory sync, content hash, and
lineage digest are verified, then releases every exact lease once and publishes
the final archive; response-loss reconciliation proves the staging receipt
before any release and proves each release receipt,
while an unknown release blocks cleanup. Start and archive check the live
provider/source against every owned/quarantine/destructive root in both
directions. Cleanup always checks the archived frozen boundaries; after verified
release it also recanonicalizes and validates a provider/source that still
exists, but accepts its external move/deletion without inventing or touching the
missing object. Unica never writes, moves, quarantines, or deletes provider
content.
Crash fixtures stop immediately before/after staging durability, each lease
release, and final archive publication. No release is legal with missing/stale
staging proof; after durable staging, recovery binds that exact receipt and
never rewrites provider content.

## State-Machine Evidence

Unit tests must enumerate every allowed and forbidden edge. Normal flow is:

```text
created -> preflightPassed -> baselineReady -> developing -> localVerified
-> synchronizationPrepared -> synchronized
-> integrationPlanned -> acquiringLocks -> locked -> mainMerged
-> mainValidated -> committing -> committedAndUnlocked
-> archivedSuccess -> cleanedSuccess
```

Only a session with conflicts branches
`synchronizationPrepared -> synchronizationConflicts -> synchronizationPrepared`.

Support-preflight outcomes are not phases. Tests prove that
`manualSupportRequired`, `vendorForbidsChanges`, and
`supportPreflightInconclusive` leave the task safely `synchronized`, expose only
the current digest-bound support handle, and cannot create a main session or
lock plan. `ready` is accepted only while every bound gate input remains current.
The manual outcome additionally exposes an `awaitingArm` action authorization,
but only the acquire-root instruction: editing or committing is still
forbidden. The exact `supportPrerequisiteArm` apply is the sole transition to
`armed`; it binds an immutable receipt after rechecking the complete unrelated
prefix, unchanged support/original/handoffs, and the bound actor's current root
ownership. Pre-arm exact drift at preview or apply final recheck never arms or
cancels and leaves the action `awaitingArm`; after releasing a held root, the
caller must run the separate fully proven cancellation. Incomplete evidence
also leaves it awaiting. Reconciliation is legal only
from `armed`, binds the expected arming receipt, and accepts only the first
root/support version after its arming cursor. Cancellation is legal in both
states but carries arming fields exactly when it follows arming. An awaiting or
armed action survives only its typed arming/reconciliation/cancellation history
scan; it is never readiness evidence. Other authoritative mutations and
abandonment reject while it is pending. In separate mode, reconciliation,
cancellation, and terminal recovery also prove an exclusive lease and exact
clean recorded-base state; known-busy and acquired-dirty observations are the
two tagged `manualSupportLocalChangesRemain` variants and perform no
terminalization. Wrong post-arm immutable history freezes the action and enters an exact
support-prerequisite plan with one of the three disposition-bound results,
complete routine/external history preservation, and a root-first finalization
guard.
After a valid manual prerequisite reconciliation the task returns to
`localVerified`; a fresh Dn, supported update, delta proof, and gate are required
before `integrationPlanned` can be reached again.

Gate-lifecycle tests distinguish `current` readiness from
`consumedByOriginalMerge`. Current history evidence must bind its partition
endpoints to the gate-observed/classified-through cursors and may extend only
over a completely unrelated tail. The root support guard is acquired first and
the current gate/evidence are rechecked while held. A non-anchor mismatch after
lock acquisition enters `staleSupportPreflight` until exact unlock; relevant
history returns to `localVerified` for fresh Dn even after a net-byte restore.
Proven original apply atomically binds the gate to its merge receipt and
authorized post-merge fingerprint; the consumed handle is legal only for that
post-merge verify/commit lineage. Commit preview and the immediate pre-effect
guard independently bind the post-merge partition endpoints and the proven
atomic safety capability. Drift starts no commit and publishes exact
restore/unlock recovery.

Safe abandonment ends in `archivedAbandoned -> cleanedAbandoned`. The blocking
states `blockedByForeignLock`, `staleRelevantBaseline`,
`staleSupportPreflight`, `unexpectedDelta`, `lockPlanExpansionRequired`,
`validationFailed`, `commitBlocked`, `recoveryRequired`, and
`committedUnverified` must have explicit exit tests. Neither
`recoveryRequired` nor `committedUnverified` can transition to archive or
cleanup. Tests use the exact exit guards/destinations in ADR-0012 rather than
merely asserting that some outgoing edge exists.

If archive `dryRun` preview finds task-only support transitions, it stops with
`manualSupportCleanupRequired` at the exact origin phase and returns only the
inverse proposal/evidence digest. It creates no authorization/archive and does
not acquire, inspect, or release an external lease. The distinct approved
archive apply rechecks that proposal, journals before any external lease gate,
and publishes the `awaitingArm` cleanup authorization only after the
recovery-handoff and mode-specific baseline gates pass. Missing/stale evidence
or a capability-proven busy/dirty baseline instead returns the typed archive
`supportPreflightInconclusive` stop, creates no authorization/archive, and
preserves the origin phase after verified release. An unknown acquire,
inspection, or release effect enters recovery rather than that stop. Reconciliation or its
recovery reaches `abandonmentReady`, which normally allows only status,
capability-proven routine refresh, and abandoned archive. A current cleanup
authorization additionally allows only its exact arming/reconciliation/
cancellation; a frozen one allows only status/recovery. A concurrent routine version can
therefore be selectively applied and reclassified without reopening development
or dead-ending the task; any remaining task-only transition issues a fresh
cleanup proposal whose distinct approved apply must repeat the same journaled
gates before publishing an authorization.

The lock-window failure tests prove all exact pre-effect exits: stale relevant
anchors retain the full owned lock set until verified unlock returns
`localVerified`; a non-anchor gate mismatch retains/compensates its exact set in
`staleSupportPreflight` until verified unlock returns `synchronized`; an expanded lock requirement retains that set in
`lockPlanExpansionRequired` until verified unlock returns `synchronized` and
invalidates the main session, verification, plan, and lock evidence. Invalid
post-original verification must first enter `recoveryRequired`; only the exact
checkpoint-restore plus full-unlock recovery proof may enter
`validationFailed`.

An observed vendor-ancestry loss after request/digest validation must enter
`recoveryRequired` with a `taskConfiguration` checkpoint restore/recreate plan;
only a proven task fingerprint may return `localVerified`. Selecting a wrong or
stale input ID is instead a rejection and cannot fabricate this domain stop.

The refresh-distribution matrix enumerates exactly `localVerified`,
`synchronizationPrepared`, `synchronizationConflicts`, `synchronized`,
`integrationPlanned`, and `blockedByForeignLock` as legal source phases and
rejects every other phase as `taskPhaseMismatch` unless a higher-precedence
safety blocker exists. Each legal row proves current local-checkpoint
verification, no pending/frozen support action/recovery plan/live worker/owned
lock/original difference/unknown effect, and a fresh clean
binding/main/database/head/permission/capability inspection; the foreign-lock
row additionally proves that exact operation's compensation and empty owned
lock set. Apply rechecks all of this, invalidates every Dn-and-later
artifact/session/decision/verification/gate/plan/preview proof, and returns
`localVerified`.

The foreign-lock exit test then proves compensation/no owned locks, uses that
fresh clean inspection and applied refresh distribution, and makes exactly one
later bounded lock observation; neither status nor the skill polls or treats
external coordination as proof of release.

Transition tests cover the guarded abandonment edge from every eligible phase
named by ADR-0012 and its rejection from lock/main-mutation/commit-ambiguity
phases. They also prove that accepted task-only support transitions cause
archive preview to issue only an inverse proposal/evidence digest and remain
unarchived; the distinct approved apply journals before external lease gates
and alone may publish `awaitingArm`. A pending action must first be
armed and reconciled or explicitly cancelled. `unexpectedDelta` can reach `adapted` only through a recorded
verification/difference-digest decision with rationale followed by a fresh
matching verification.

Cancellation-path tests prove `locked` plus unchanged original can reach
`synchronized` only through complete `repository.unlock(reason="abandonment")`.
From `mainMerged`/`mainValidated`, the first archive preview must return the
exact no-effect abandonment recovery plan; only approved checkpoint restore,
before-state proof, and full unlock through `repository.recover` may return to
`synchronized` for a second, eligible archive preview. Commit ambiguity has no
such path.

Pending-plan tests prove status exposes exactly one current plan and every other
mutation is rejected. The no-effect abandonment preview can be cancelled only
through the digest-bound local recover-cancellation variant, which leaves the
main phase/anchors unchanged and invalidates the plan before normal
verification or commit resumes; effect-started recovery cannot be cancelled.

Each public tool that changes phase asserts its pre-state and writes its
postcondition before publishing success. `unica.merge.verify` owns the local,
synchronized-task, pre-lock main-sandbox, and post-merge main-integration
validation gates; it reruns the configured checks and stores receipts rather
than trusting prose supplied by a caller.

## Preflight And Lock-Window Evidence

Preflight records exact platform/configuration/repository identities, original
topology and repository transport, binding, compatibility mode, cleanup and
commit-comment policies, required manual-support target mode and conditional
actor/history identity, separate-mode inspection/exclusive-lease service
capability, standard version, and optional Git branch/commit. Git context is
evidence only and never the merge base.

Main-integration support preflight records the ordinary-result and comparison
IDs, complete canonical candidate/support-layer list, repository/original
anchors, sandbox/settings/support-graph/capability digests, one closed outcome,
endpoint-bound history evidence, the verified ordinary per-layer
recovery-distribution set for any manual authorization, the exact task-scoped
retention-provider lease/receipt and provider-object handoff for each such
layer, the initial `awaitingArm` authorization plus root-acquisition instruction,
and any exact external action. No manual edit instruction exists before the
separate arming apply proves the actor-owned root and unchanged bound evidence.
A real no-force
sandbox operation plus support-graph evidence must prove completeness; a
candidate list inferred from XML files, CFU, or a localized message alone is
rejected. A current `ready` gate and current history-evidence digest are required
ancestors of main-sandbox verification, lock plan, acquired set, and original
merge. On successful original apply its receipt/post-fingerprint-bound consumed
form, not a newly current gate, is the required ancestor of endpoint-bound
post-merge verification and final commit evidence.

Repository-update evidence separately proves the complete history partition,
exact root-first target closure, selective `-Objects` request, applied object and
reference fingerprints, reverse release, and post-release partition. Tests must
fail any implementation that substitutes a claimed version-pinned selector or
uses a broad whole-configuration fallback.

For a pending manual action, every arming/reconciliation/cancellation preview
partition's `fromExclusive` is exactly the authorization cursor. Arming apply
keeps that lower bound, proves an all-unrelated prefix, and records its endpoint
as the immutable arming cursor; reconciliation requires the authorized version
to be the first root/support version after it. Subsequent apply/frozen-recovery
evidence keeps the authorization lower bound and classifies a gap-free range
through the guarded observation; the post-release partition begins at the
proven pre-effect boundary. The mode-specific exclusive IB lease closes the
local-configuration race that a repository root lock alone cannot close.

Before any repository lock, the synchronized task IB must have immutable
receipts for configured unit, integration, feature, data-migration, diagnostics,
syntax, and maximum safe local checks. While locks are held, only bounded
configuration/UUID/reference/delta/diagnostics/ownership checks run. The
invocation transcript must prove no `/UpdateDBCfg`, database restructuring, or
destructive runtime test targets the original infobase. A validation failure
uses the exact rollback/unlock exit and returns substantive repair to the
disposable task.

## Archive And Cleanup Schema

The versioned compact archive contains exactly the retained evidence classes:

- task manifest, external task/internal instance IDs, state transitions, and
  operation input/result digests;
- standard/platform/capability-row versions and original/repository identities;
- repository anchors and SHA-256 of D0, every refresh Dn, and ordinary result;
- pre/post canonical deltas and platform comparison reports;
- merge settings, typed conflict decisions, and manual change receipt digests;
- support-layer audit, every support-preflight/action outcome/digest, the full
  external prerequisite/recovery version chain with target mode,
  actor/working-IB identity, transitions, root/conditional actor-lock proofs,
  `awaitingArm`/`armed` state, immutable arming receipt/cursor, arming-time root
  observation, and exact actor/IB/delta/version-order proof, mode-specific IB
  lease plan/proof, authorization-anchored history partitions,
  recovery-distribution evidence, provider-object handoff plus retention acquire/
  revalidation/release receipts, all three recovery dispositions, preserved routine/proven
  external-support observations, exact exceptional content restorations,
  finalization guards, cancellation/recovery receipts, and proof that technical
  support did not enter original;
- lock plan plus acquisition/compensation/rollback journal;
- local, synchronized, main, and commit validation receipts;
- commit comment, exact object set, endpoint-bound post-merge history evidence,
  atomic-safety capability ID, repository version/evidence, content proof, and
  released-lock proof;
- bounded redacted diagnostics and a manifest hash over every archive entry.

It excludes CF/XML/source bytes, database files, checkpoints, secrets or secret
hashes, raw logs, raw argv/stdout/stderr, and unbounded platform reports.

Successful or safely abandoned cleanup removes only the marker-owned task File
IB, XML workspace, probe/merge sandboxes, checkpoints, transient distribution
and ordinary CF artifacts, staged dumps, and non-archived logs. The compact
journal/archive remain. Every removed role appears in the preview and terminal
cleanup receipt; no unlisted path is traversed. A profile-owned recovery CF and
its retention-provider root are never cleanup roles. Archive may drop their
task reference only after the exact retention lease is proven released; an
ambiguous release prevents cleanup. Both preview and apply compare every live
owned/quarantine/destructive target with the archive's frozen canonical
provider-root/source boundaries. If an external provider/source still exists,
its live canonical identity is revalidated and any changed identity or
equality/ancestor/descendant overlap prevents cleanup as `unsafeTaskPath`. If
the external owner moved or deleted it after verified archive-time release,
cleanup succeeds using the frozen boundary and performs no provider operation.
Acceptance tests cover both live revalidation failure and successful
move/delete-after-archive cleanup without a retention-provider call or any
provider-content write, move, quarantine, or delete.

## Capability Fixture

The real fixture is opt-in and destructive only inside an explicit owned root:

```sh
UNICA_BRANCHED_ACCEPTANCE_CONFIG=/absolute/path/profile.yaml \
  cargo test -p unica-coder \
  --test platform_real_branched_development \
  -- --ignored --nocapture --test-threads=1
```

The profile names an exact Designer executable, empty test root, exact original
infobase kind, repository transport, disposable server provisioning when the
platform row is client/server, and test credentials through environment
references. The harness refuses root/home/Git paths and creates a unique marker-owned child, two
repository users, one disposable repository, a separate manual working IB, and
owner-only inspection/exclusive-lease services for the original and separate
manual working IB, plus a lease-backed WORM retention-provider root disjoint from
all owned/protected roots and
local File task/probe/sandbox IBs. It executes separate profile instances for
both explicit manual target modes. The original fixture is File or disposable
client/server according to the row; repository transport is independently file
or server. The valid source resolves below that provider root; malicious-profile
cases escape it through traversal/symlink/reparse or nest the provider root and
each owned/protected root inside the other in both directions, and must fail
before destructive work.

The harness emits two independently validated evidence products. The platform
fixture row is written only to
`plugins/unica/references/branched-development/platform-capabilities.json` and includes the
exact `manual-recovery-handoff-readability` case proving Designer opens the
bound ordinary CF/SHA as the manual actor. The retention fixture row is written
only to
`plugins/unica/references/branched-development/retention-provider-capabilities.json`;
its closed evidence records the
exact acquire/replay, held observation, bound-SHA actor readability, held
rename/overwrite/delete denial, exact-once release/replay, unknown-effect
reconciliation, and canonical path/symlink/reparse cases. Each row has its own
contract/harness/evidence digests and pass status. Tests reject cross-manifest
IDs and prove that neither row can substitute for the other.

It records platform version, host OS/architecture, locale/encoding, topology,
contract digest, exact operation-class timeout map, implementation commit,
command result, redacted service
messages, artifact hashes, and every postcondition. It preserves the fixture on
an unknown effect and prints the recovery path; otherwise it quarantines and
deletes only its marked child. One topology's evidence never enables another.

One serialized run must cover:

1. ordinary CF versus distribution CF classification and D0 deployment;
2. precise existing-object support edit in unsupported and multi-vendor chains;
3. one-sided and twice-changed scalar and module updates;
4. reproducible `takeOurs`, `takeTheirs`, `combine`, and typed manual replay;
5. add/add UUID/name collision and top-level UUID preservation;
6. new form/attribute ownership and lock planning;
7. delete/modify and reference-cleanup planning without forced clearing;
8. foreign lock, bounded/hung-call timeout, partial acquisition, reverse
   compensation, and owner parsing;
9. same-user pre-existing lock behavior under the exclusive-user contract;
10. relevant and unrelated repository advancement with endpoint-bound history
    partitions, root-first exact target locking, selective `-Objects` refresh,
    and capability-proven add/delete structural confirmation, proving no
    version-pinned selector or broad fallback is used;
11. main merge support isolation and scoped fingerprints;
12. complete support-preflight outcomes for already editable, human-editable,
    vendor-forbidden/off-support-required, and incomplete-diagnostic cases;
13. both manual target modes, lost-response action reconstruction, explicit
    no-action cancellation before and after arming, acquire-root-without-edit ->
    repeatable read-only `supportPrerequisiteArm` preview -> digest-bound
    `localJournaled` apply -> instructed root retention and exact
    actor/IB/delta/version-order commit/release, missing/wrong-root,
    `stage=preview`/`stage=applyRecheck` preserving `awaitingArm` before separate
    cancellation, inconclusive evidence, apply replay/lost-response,
    first-root/support-version, and authorization-cursor anchored arm/
    reconciliation/cancellation/frozen partitions, reserved-original and
    separate-working-IB lease clean/busy/dirty/unknown outcomes (including
    unknown post-authorization lease effects retaining armed
    `supportPrerequisite` recovery or awaiting-cancellation
    `preArmSupportCancellation` recovery), root-only normal support versions and exceptional exact
    invalid-content restoration, optional read-only root observation plus
    mandatory guarded apply, reserved-actor inventory, retained-lock stops, all
    three recovery dispositions with routine/disjoint-external preservation and
    root-first finalization, versionless `originalNotClean`, prior-unattributed-
    only ownership reclassification, all three `DeferredRepositoryAdvance`
    variants with preview preservation/exact approved-apply consumption,
    inverse-abandonment cleanup proposal/apply staging with a no-lease preview,
    pre-gate journaled apply, typed known blockers, and unknown-effect recovery, fresh
    Dn/rebase, support-graph guard/recheck, consumed-gate lineage, and release;
14. rollback/restore/unlock for modify, customer-owned delete, interruption,
    and failure; vendor-object deletion requiring `offSupport` must stop;
15. one integration-set commit with frozen task-bound comment, endpoint-bound
    post-merge history evidence, unrelated-concurrent success, capability-proven
    atomic rejection of a concurrent deletion-blocking referrer with no partial
    commit, plus harmless closure expansion recorded as
    `nonConflictingConcurrent`,
    failure/ambiguity cases, and released-lock proof;
16. successful archive/cleanup, safe abandonment, exact-once retention-provider
    lease release after durable handoff lineage, acquire/replay/release response
    loss, WORM rename/overwrite/delete denial, refusal on unsafe or nested
    provider/source/owned paths, live-provider revalidation after release, and
    successful cleanup after the external owner moves or deletes the released
    source without any provider-content mutation;
17. restart or worker death at every dangerous journal boundary, including task
    deployment and task supported-update apply before/after each effect marker.

The completion suite repeats applicable scenarios for File and client/server
originals and for every file/server repository transport claimed by the
platform capability manifest. Each claimed provider/host/storage combination
is independently covered by the retention-provider capability manifest.

A row in either capability manifest is invalid if any required case is skipped
or verified only interactively without retained machine-readable evidence.

## Fake and Pure Test Layers

Required pure suites:

- transition table and terminal guards;
- operation replay and input-hash mismatch;
- atomic journal/schema migration and every write barrier, with exactly
  `registered`, `intentWritten`, `effectUnknown`, and `terminal` operation
  states; crash after the fsynced observed barrier reconstructs terminal output
  without replay;
- stable project/target/account identities, collision/relocation rules,
  cross-state-root original/account/task/session leases and reservations,
  two-host same-target and shared-account contention, persistent fenced
  observe/renew/release receipts after lease loss, and fail-closed unproven
  coordinator topology, non-overridable target locators, owner-only permissions,
  start-attempt replay, and unresolved-task exclusion;
- task ID normalization, pairwise work-root/state/coordination/original-workspace/
  File-IB/file-repository non-overlap, and destructive path/marker
  target-identity policy;
- typed secret/value redaction;
- canonical UUID/property delta and unsupported-kind rejection;
- metadata ownership and reference closure for modify/add/delete;
- layer-aware support parser/editor round trip;
- support-candidate closure and deterministic four-outcome precedence, including
  XML-path non-authority, CFU reject-only classification, and
  `offSupport`-requiring vendor deletion;
- both manual target-mode presence rules, external prerequisite version
  attribution, exact root semantic delta, root/conditional reserved-actor lock
  proof, reserved-original and separate-IB lease clean/busy/dirty/unknown
  outcomes, `awaitingArm`/`armed` transitions, acquire-root-only instruction,
  repeatable handle-free arm preview plus `localJournaled` apply receipt replay,
  missing/wrong-root and staged `stage=preview`/`stage=applyRecheck`/gap/
  inconclusive outcomes, stale-state preservation plus separate cancellation,
  release/reacquire without intervening root/support history, and
  first-root/support-version after
  the arming cursor, pending-action response-loss reconstruction/cancellation
  before and after arming, exact authorization-cursor anchors across arm/
  reconciliation/cancellation/frozen history, versionless
  `originalNotClean`, prior-unattributed-only ownership reclassification, and
  all three `DeferredRepositoryAdvance` variants, no invented version for
  `coverageUnknown`, preview preservation, exact approved-apply consumption,
  and terminal non-reopening, three disposition-bound invalid-history
  recoveries preserving routine/disjoint external support, exact
  finalization guards, pre-arm crash-stable guard capability, one-effect/one-
  receipt action mapping, stage-sensitive recheck/replan versus protected-boundary
  breach, compensated attempt audit, known root/mode blocker, terminal receipt/
  status/archive equality, inverse-abandonment no-lease proposal preview plus
  journaled approved apply/typed-stop/recovery staging, invalidation to
  `localVerified`, and fresh-Dn lineage;
- deny-unknown retention-provider/profile parsing; separate manifest/ID
  resolution; missing/stale/provider-host-storage-mismatched rows; skipped,
  duplicate, digest-mismatched, or unpassed case evidence; exact
  provider-object/source/SHA binding below the provider root; provider-root
  versus owned/protected-root containment and source traversal/symlink/reparse
  rejection; idempotent task-lease acquire/replay; exact held observation;
  bound-SHA readability; WORM rename/overwrite/delete denial; unknown-effect
  reconciliation; corrective-time revalidation; archive-before-release;
  exact-once release replay; ambiguous-release cleanup refusal; still-live
  identity revalidation; and successful post-release external move/delete
  cleanup using only the frozen provider boundary;
- ready-gate root-first recheck, stale compensation, authorized original-merge
  consumption, exact history-partition endpoint and semantic-digest mapping,
  complete typed expected/observed mismatch evidence for every gate input,
  `staleSupportPreflight` entry/unlock exit, and receipt/post-fingerprint-bound
  verify/commit lineage including immediate pre-commit drift recovery;
- selective repository-update planning/apply with root-first exact target locks,
  `-Objects`, structural add/delete confirmation, post-release partitions, and
  explicit rejection of a version-pinned-selector or whole-configuration claim;
- post-merge endpoint-bound partitioning and capability-proven atomic commit
  rejection of a new deletion-blocking referrer before any task-content effect,
  plus harmless closure expansion classified as `nonConflictingConcurrent`;
- exact per-tool schema/envelope/data/evidence/error contract, including closed
  missing-task reservation blockers/contexts and no-task busy rejection;
- canonical JSON/digest vectors: `[]` ->
  `4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945`,
  `{"b":2,"a":1}` -> canonical `{"a":1,"b":2}` ->
  `43258cff783fe7036d8a43033f830adfc60ec037382473548ac742b888292777`, and
  record-without-`evidenceDigest` -> canonical `{"a":"€","z":null}` ->
  `31eb5e4b861ebb6f087b97caf4f3009898b494ee322dd037ac9fa57a330b5315`;
  duplicate/non-I-JSON rejection and different tool/policy operation-digest
  non-replay are mandatory;
- read-only, proven-contained, and authoritative/unknown-effect timeout result,
  phase, durability, and recovery behavior; read-only storage/schema assertions
  prove no operation/lease/start-attempt/receipt/durable handle/task mutation,
  and only pre-existing mutating records may be referenced;
- ordinary branched-task mutation changed/no-change receipts, exact closed
  phase-transition pairs for every legal task-mutation phase, rejection of
  every other `TaskPhase`, no-op same-phase/evidence/cache preservation and
  replay, exact changed descendant-evidence invalidation, and the fixed
  `synchronizationConflicts -> synchronizationConflicts` pair for both
  resolution outcomes;
- same-target resolution-receipt supersession without cross-target expiry,
  generation-scoped manual-resolution receipt expiry, and decision replacement:
  after decision D1 consumes receipt R1, a new changed receipt R2 for that
  target keeps R1 immutable/consumed, moves D1 to `replacementPending`, and a
  CAS-bound D2 carries `replacesDecisionId=D1`. Status and conflict data expose
  the exact head transition, repeated same-target edits carry the pending head,
  response replay repeats no transition, and resolved replay applies only D2;
- worker socket/pipe token, PID/start-nonce binding, protocol, status, and
  policy-allowed cancellation;
- task-configuration recovery plans that restore/recreate from the exact
  checkpoint and never accept or blindly replay an unknown task mutation;
- merge settings, localized conflicts, decisions, and manual replay;
- resolved-replay refusal before every conflict is decided and before every
  resolution-workspace change is bound, including deterministic primary-code
  precedence when both defects exist;
- digest-bound supported-update session replacement after accidental unbound
  resolution edits, proving create-before-invalidate atomicity and expiry of the
  old workspace, decisions, and receipts;
- staged task-source publication, atomic replacement, cache events, and
  task-IB/XML fingerprint equality after authoritative replay;
- relevant anchors, lock planning, compensation, rollback, and commit
  reconciliation.

The fake Designer suite injects deterministic exits, output encodings,
twice-changed output, unresolved references, and their simultaneous case in
which `twiceChangedProperties` is `stopCode`/`errors[0].code` while one
`MergeSessionData` retains every conflict, rejected merge settings, a hung read-only
`delivery.inspect` and capability-proven repository inspection, a hung contained
sandbox call, a hung lock process, partial lock success, compensation success,
compensation failure, a crash after a lock effect but before its
observed-journal write, stale anchors, extra lock requirements, secret-bearing
diagnostics, rollback success/failure, commit success, concurrent-referrer
atomic rejection before effect, complete/partial/unclassifiable support-block
diagnostics, both manual target modes, reserved-original and separate-IB lease
busy/dirty/unknown, unknown post-authorization separate-IB lease effects that
retain the frozen support-prerequisite lineage, retention acquire/replay/release
loss and overwrite/delete denial, acquire-root-only, repeatable arm-preview
response loss, arm-apply replay/response loss, missing/wrong root, both staged
outcomes for pre-arm support/original/handoff/history drift preserving
`awaitingArm`, subsequent explicit cancellation, inconclusive evidence,
release/reacquire without an intervening root/support version, and first-root/support-version
violations, wrong-mode/actor/IB or non-root support prerequisites, retained
manual locks, pending cancellation before/after arming and authorization-anchor
gaps, crashes at every pre-arm cancellation guard/lease/update/authorization
barrier with observation-only recovery, digest-changing reapproval, and
no-arming finalization, versionless original-dirty, forbidden positive-observation ownership
reclassification, all three deferred-advance variants with no-invented-version
coverage, preview preservation, and approved-apply-only consumption, all
recovery dispositions/finalization
blockers, stale/consumed support-gate inputs and `staleSupportPreflight`, task-deploy/task-
apply death at every journal barrier, partial commit, ambiguous commit exit,
and unlock failure after commit.
Each boundary is a separate case; read-only timeout must prove termination,
temporary-output disposal, unchanged phase, and absence of durable operation or
recovery state. One generic partial-effect test cannot satisfy several. The
suite verifies domain behavior but cannot create a capability manifest row.

OS-specific locator, process, filesystem, encoding, and reparse tests live under
`infrastructure/platform/**` or `tests/platform/**` so CI classification runs
the macOS/Linux/Windows matrix.

## Package and Skill Acceptance

The packaged skill is
`plugins/unica/skills/branched-development/SKILL.md` with public name
`unica:branched-development`. It is product-owned and routes only through the 21
public lifecycle tools, the layer-aware `unica.support.edit mode="layer"`, and
installed typed Unica tools whose exact operation variants advertise
`supportsBranchedTask`. It may use `unica.code.patch` or another compatible BSL
writer when advertised, but does not require one globally. Without a compatible
writer it stops only before an actual BSL mutation/manual BSL conflict and does
not substitute another mutation channel. The
legacy broad support-edit mode is explicitly forbidden. The skill never
instructs the model to invoke Designer, `v8-runner`, shell, or a packaged
operation script directly.

For local development the transcript passes the original project `cwd` plus the
opaque `branchedTask` context selecting either `taskWorkspaceId` or an exact
session/digest-bound merge-resolution workspace. It never computes, prints, or
substitutes either disposable workspace path. Ordinary task mutations prove
their exact legal phase pair, phase rollback and descendant-evidence
invalidation; manual/combine receipts cannot escape their session. If a later
same-target edit follows a consumed conflict decision, the transcript shows the
old immutable receipt/decision, `replacementPending`, the new decision's exact
`replacesDecisionId`, and replay of only the final current head.

The ordered E2E transcript must prove:

```text
status/recover -> start/preflight -> repository-clean D0 -> disposable work
-> local verification -> current D1 -> supported rebase/conflicts/delta proof
-> ordinary result -> main support preflight
-> [if manual: profile-bound human acquires only the configuration root
    without editing or committing
    -> repeatable handle-free supportPrerequisiteArm read-only preview
    -> localJournaled apply with approvedArmingDigest
    -> follow the retain-root instruction, edit only armed transitions,
       commit/release the exact actor/IB/delta version as the first root/support
       version after the arming cursor, with no intervening root/support version
    -> close the bound Designer session
    -> status/reconcile under the mode-specific exclusive IB lease
    -> fresh Dn/rebase
    | if no action: release any human root, close the bound Designer session,
      then explicit cancellation from awaitingArm or armed]
-> ready main sandbox -> lock plan -> root-first guarded acquisition
-> relevant-anchor check -> bounded original merge
-> merge.verify(mainIntegration) -> one task-content commit/release
-> archive -> cleanup
```

It must also prove that the skill stops on all three non-ready support outcomes,
`manualSupportLocalChangesRemain` in both lease-busy and lease-acquired-dirty
forms, every frozen-recovery pending/lock outcome, `staleSupportPreflight`,
foreign locks, unknown effects, unresolved conflicts, unsupported change kinds,
and incomplete cleanup proof.
For an external dependency it emits terminal blocking guidance and does not ask
the user a generic “continue?”/confirmation question or poll automatically.
After external coordination it starts with `unica.branched.status`, then refreshes
repository/distribution/plan instead of reusing stale evidence.
For `manualSupportRequired` it names only the exact forward candidates and/or
task-surplus restore transitions,
states the profile-bound `reservedOriginal` or `separateWorkingInfobase` target
and exact actor/IB instructions. The first stop tells the human only to acquire
the configuration root without editing or committing and then run the typed
arming update. The arm preview is strictly read-only and repeatable after
response loss; it takes no `operationId`/`dryRun` and creates no durable preview
handle. Only the `localJournaled` apply with `approvedArmingDigest` may publish
the edit instruction. That instruction asks the human to retain the root, apply
only the armed transitions, commit/release a separate version, and then close
Designer before status/reconciliation. The skill explains that acceptance is
based on the exact actor/IB/delta, no intervening root/support version, and the
commit being the first root/support version after the arming cursor; an otherwise
exact release/reacquire is not rejected merely for lacking continuous-lock
evidence. It explains that root
commit success is not enough: Unica must reconcile the version and repeat
distribution/synchronization/preflight. Status must reproduce the exact
transition list, authorization state, and arming receipt after a lost response.
If the human elects not to edit, the skill requires release of any held root and
routes to the typed cancellation appropriate to `awaitingArm` or `armed`.
For either target it explains the exclusive-lease/clean-close requirement
without exposing the service secret or claiming Unica will reset local changes.
If recovery needs a vendor CF, it shows only the safe retained
handoff name/reference and never a provider path, endpoint, token, or object
handle. For frozen history it presents the exact
disposition, preserved external/routine chain, corrective/conflict instruction,
and finalization locks. It never asks the user to press a generic “Continue”. If
abandonment requires inverse support cleanup, the transcript first shows the
no-lease proposal preview, then the distinct approved archive apply and its
typed gate result; it shows the exact root-only action only after that apply
publishes `awaitingArm` and never claims archive completion first.

Before the corresponding action, the transcript shows the user:

1. the exact support layer/object/rule that will become editable;
2. the support-preflight outcome and, when manual action is required, the exact
   root-only prerequisite instructions plus the separately recorded version;
3. every foreign lock, nullable proven owner, redacted diagnostic, and requested
   external coordination;
4. every manual/combine conflict decision and rationale;
5. every expansion of the lock plan and its reasons;
6. the exact commit object set, validation/history-evidence digests, atomic-
   safety capability, and comment;
7. cleanup eligibility, outcome, archive ID, and an opaque owned-target locator
   that does not expose the absolute disposable path.

The skill may report repository success only at `committedAndUnlocked`. It may
report full task completion only at `cleanedSuccess`, together with archive
location/hash and repository evidence. `cleanedAbandoned` is reported only as a
safely abandoned task and never as issue completion.

Package checks must include:

- all 21 tools in source and generated-package `tools/list`;
- required layer-aware support-edit schema/round-trip;
- conditional BSL-writer coverage: an advertising package proves the exact
  branched-task/resolution schema, durable change/no-change receipts,
  no-op/replay, and manual-conflict binding; a package without a compatible
  writer proves that only BSL-writing scenarios stop before mutation and that
  no shell/direct-file fallback appears;
- an exact generated `supportsBranchedTask` registry snapshot covering the
  concrete project/configuration reads, required layer-aware support mutation,
  every advertised and selected code/metadata/form mutation, configuration
  build/load, diagnostics/syntax, and test operations used by the skill, with
  rejection tests for every other tagged operation;
- exactly one `.mcp.json` server named `unica`;
- `branched-development` in skill scenario and product-source provenance
  validation (`sourceKind`, owner repository, design path; no fabricated donor);
- no skill-local runtime scripts or raw Designer guidance;
- generated schemas/status transcript cover both manual target modes, both
  `manualSupportLocalChangesRemain` lease variants, exact transition
  reconstruction, acquire-root-only -> repeatable handle-free preview ->
  `localJournaled` arm apply -> exact actor/IB/delta/version-order commit,
  `awaitingArm`/`armed` status and arming-receipt replay, missing/wrong-root,
  staged `stage=preview`/`stage=applyRecheck` preserving `awaitingArm` before
  separate cancellation, inconclusive outcomes, first-root/support-version enforcement,
  authorization-anchored arm/reconciliation/cancellation/frozen history,
  versionless `originalNotClean`, constrained ownership
  reclassification, all three `DeferredRepositoryAdvance` variants with
  no-invented-version coverage, typed incomplete-preview stop, complete-preview
  preservation, approved-apply-only consumption, all support-recovery
  dispositions/finalization receipts, provider-handoff retention acquire/
  revalidation/archive-release lineage, cleanup proposal/approved-apply
  authorization staging including known stops and unknown-effect recovery, current/consumed
  gate-history endpoints, and post-merge atomic-safety lineage;
- generated-package smoke on every release target;
- a byte scan showing that fixture secrets are absent from retained evidence;
- a recursive byte/value scan over every compatible general-tool response field
  proving that absolute task/work-root/state/coordination paths and path-bearing
  diagnostics never cross MCP.

## Completion Audit

Before closing #137, enumerate BD-01 through BD-28 and attach the authoritative
test/evidence path for each row. A green narrow suite cannot support a broader
row. Platform-version detection, documentation, fake output, or lack of a found
failure is not proof of a real repository postcondition.
