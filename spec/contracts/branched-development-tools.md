# Branched Development Tool Contract

## Status And Scope

This is the normative semantic request/response contract for the 21 public
tools in
[ADR-0012](../decisions/0012-safe-branched-development-for-1c-configuration-repositories.md).
The implementation publishes generated JSON Schemas and commits their exact
snapshots under
`tests/fixtures/branched_development/tool_schemas/<tool>.json`; those snapshots,
Rust request/result types, and contract tests become the executable schema
contract before any handler is registered. They use camelCase,
`additionalProperties: false` recursively, bounded strings/collections, and
`oneOf` tagged unions. No free-form object or untyped array is allowed.

The record notation below is normative for required fields and enums: `?`
means optional and `[]` means a typed collection. Fields not shown are rejected.
`cwd` is the one caller-supplied path: it selects the original project workspace
and is resolved by normal workspace policy. Designer executables, repository
locations, task roots, artifacts, checkpoints, and sandboxes come only from
local profile or registered IDs.

`unica.code.patch` and layer-aware `unica.support.edit` are separately owned
general developer tools required by the packaged workflow. They are not counted
among these 21 lifecycle tools.

Their minimum companion contract is explicit:

- `unica.code.patch` is the tool owned by issue #73. In a task workspace it
  returns a durable `changeReceiptId`, affected object/property identity, before
  and after SHA-256, and cache/event evidence. The branched workflow rejects a
  code-patch implementation that cannot bind a manual merge decision to that
  receipt.
- `unica.support.edit` accepts `mode: "layer"`, `layerId`, and exactly one of
  `{ operation: "setCapability", enabled }` or
  `{ operation: "setObjectState", objectId, state }`, where state is `locked`,
  or `editable`. `offSupport` is absent from the branched-task variant and is
  rejected. It supports an honest dry-run and returns a layer/object
  before/after receipt plus proof that all unrelated layers are byte-identical.
  Legacy broad `Capability` or unscoped UUID matching is rejected when a
  branched task is active.

## Common Types And Schema Bounds

- `TaskId`: 1-64 ASCII characters matching
  `[A-Za-z0-9][A-Za-z0-9._-]{0,63}`.
- `OperationId`: canonical UUID string.
- `ProjectId`: canonical UUID from local project configuration.
- `UnicaId`: canonical UUID allocated by Unica and scoped to one task. This is
  used by instance, workspace, infobase, artifact, probe, comparison, manifest,
  checkpoint, session, decision, verification, validation-receipt, plan,
  integration-set, lock-set, operation-receipt, archive, and quarantine IDs.
- `MetadataObjectId`: canonical metadata UUID from the 1C configuration; Unica
  does not allocate it.
- `SupportLayerId`: 1-256 printable non-control Unicode characters, returned by
  support inspection and compared exactly.
- `CapabilityRowId`: 1-128 ASCII characters matching
  `[a-z0-9][a-z0-9._-]{0,127}`.
- `RepositoryVersion`: 1-128 printable non-control Unicode characters; it is
  opaque and never parsed as an integer by the domain.
- `RepositoryOwnerIdentity`: closed `{ username, computer, infobase,
  lockedAt }`; `username` is a proven non-empty repository username and the
  other three required fields are a typed non-empty string/RFC 3339 timestamp
  or explicit `null`. When no username is proven, the enclosing owner field is
  `null`; diagnostics never infer one.
- `Sha256`: 64 lowercase hexadecimal characters.
- `DigestApproval`: `{ "digest": Sha256, "decision": "apply" }`.
- `OwnedTargetLocator`: `{ projectId: ProjectId, instanceId: UnicaId, role }`,
  where `role` is `instanceRoot`, `taskInfobase`, `taskWorkspace`, `probe`,
  `sandbox`, `artifact`, or `quarantine`; it is a logical locator, never a path.
- `ArtifactRole`: `baselineDistribution`, `refreshDistribution`, or
  `ordinaryResult`.
- `ArtifactKind`: `configurationDistribution`, `ordinaryConfiguration`,
  `configurationUpdate`, or `invalidArtifact`. `configurationUpdate` exists only
  so verification can classify/reject a CFU; no workflow input accepts it.
- `AcceptedArtifactKind`: `configurationDistribution` or
  `ordinaryConfiguration`.
- `ConfigurationIdentity`: `{ metadataUuid: MetadataObjectId, name,
  vendor, version }`; name is 1-256 characters and vendor/version are explicit
  0-256 character strings rather than omitted/inferred fields.
- `TargetKind`: `task` or `original`.
- `OriginalInfobaseKind`: `file` or `clientServer`.
- `RepositoryTransport`: `file` or `server`.

Schema snapshots use these exact general bounds unless a field below is more
restrictive: names 1-256 characters, summaries 1-2048, task summaries/comments/
reasons/rationales 1-4096, display paths 1-4096, property paths 1-2048, and
redacted diagnostics 0-8192. Control characters are rejected except normalized
line feeds in narrative fields. General result arrays contain at most 1024
items; metadata object/property/reference collections contain at most 100000.
Every nested collection item is a named closed `$defs` record in the committed
schema snapshot.

Every request requires `cwd` and `taskId`. `cwd` must resolve to the workspace
whose `unica.local.yaml` contains the task's `projectId`; it never selects the
disposable workspace or an external artifact. Every mutating request also
requires `operationId`. Per-tool field lists below omit these common fields only
when they say “no fields beyond common”.

Compatible general Unica tools use an additional exact context instead of a
task path. It is a closed `oneOf`:

```json
{
  "cwd": "/original/project",
  "branchedTask": {
    "taskId": "TASK-142",
    "taskWorkspaceId": "381f2188-554e-4abf-8cba-495a4570c5cd"
  }
}
```

or, only for a prepared manual/combine conflict workspace:

```json
{
  "cwd": "/original/project",
  "branchedTask": {
    "taskId": "TASK-142",
    "mergeResolution": {
      "sessionId": "1f99e816-1d18-4265-bdf0-204918f47c48",
      "workspaceId": "73c40b15-1ca0-4cbb-a8a8-01b28d38bfd5",
      "expectedBaseSessionDigest": "3333333333333333333333333333333333333333333333333333333333333333"
    }
  }
}
```

Their descriptors explicitly declare `supportsBranchedTask`; a tagged general
tool also declares the exact supported operation variants. The generated
registry snapshot is the release manifest for this compatibility, so enabling
one safe `runtime.execute` operation cannot silently enable all variants. The
application resolves the ID through durable state, verifies the owned marker,
phase, and leases, and supplies the disposable `WorkspaceContext` internally.
Compatible reads accept `branchedTask` but require neither `operationId` nor a
change receipt. Compatible mutations accept it, additionally require
`operationId`, and return `BranchedChangeReceipt { changeReceiptId,
contextKind, beforeSha256, afterSha256, eventIds[], invalidatedEvidenceIds[],
resultingPhase?, baseSessionDigest?, workspaceGenerationId?, receiptSequence? }`
plus cache impact for the resolved workspace. The three resolution-only fields
are required together for merge-resolution mutations and absent for ordinary
task mutations.

Before a compatible general-tool result crosses the public MCP boundary, a
recursive branched-result projection examines every field, including
`changes`, `artifacts`, `summary`, `warnings`, `errors`, `evidence`, `data`, and
`cache`. Task/work-root/state/coordination absolute paths are replaced with
task-workspace-relative logical paths or registered artifact IDs and roles;
secret values and path-bearing platform diagnostics are redacted. No absolute
disposable path may remain in either a structured value or free text. A tool
whose closed response cannot be projected without losing required semantics
must not declare `supportsBranchedTask` and returns
`toolNotBranchedCompatible` before dispatch.

An applied task-workspace mutation in `developing` keeps that phase. In
`localVerified`, `synchronizationPrepared`, `synchronizationConflicts`,
`synchronized`, `integrationPlanned`, `blockedByForeignLock`,
`unexpectedDelta`, or safely rolled-back `validationFailed`, it atomically
returns to `developing` and invalidates all
descendant checkpoints, artifacts, sessions/decisions, verifications,
integration/lock plans, and previews. It is rejected while a worker, owned lock,
original difference, or unknown effect exists and in every commit/archive/
cleanup terminal phase. A resolution-workspace mutation is allowed only for the
named live conflict session, changes no task phase/source, and produces a
receipt bound to immutable `expectedBaseSessionDigest`; decisions evolve a
separate decision-set digest and do not stale other receipts from the same
workspace generation. Recreating the sandbox invalidates the workspace and
every unconsumed receipt. A direct task path in `cwd`, an unknown ID, or an
incompatible tool is rejected.

The packaged full-cycle transcript requires compatible project/configuration
reads, typed code/support/metadata/form mutations, configuration build/load,
syntax/diagnostics, and configured test execution. Each selected concrete tool
or operation appears in the registry snapshot; generic family-name claims do
not satisfy compatibility.

The registry also classifies each compatible mutation as either an atomic
workspace-source mutation or an `authoritativeTaskConfigurationMutation`.
Atomic source edits use their durable change receipt. A build/load/runtime
variant that can leave the task IB partially changed uses the same
operation-worker and `taskConfiguration` recovery contract as delivery deploy
and task `merge.apply`; merely tagging the tool `supportsBranchedTask` cannot
downgrade that boundary.

`notCreated` is a read/preflight status, not a persisted task phase. Persisted
task phases are:

```text
created preflightPassed baselineReady developing localVerified
synchronizationPrepared synchronizationConflicts synchronized
integrationPlanned acquiringLocks locked mainMerged mainValidated committing
committedAndUnlocked archivedSuccess cleanedSuccess
blockedByForeignLock staleRelevantBaseline lockPlanExpansionRequired
unexpectedDelta validationFailed
commitBlocked recoveryRequired committedUnverified
archivedAbandoned cleanedAbandoned
```

## Response Envelope

Every task-bound success response contains:

```json
{
  "ok": true,
  "resultKind": "completed",
  "taskId": "TASK-142",
  "status": "developing",
  "summary": "Task workspace is ready",
  "changes": [],
  "warnings": [],
  "errors": [],
  "artifacts": [],
  "cache": {},
  "evidence": {},
  "data": {}
}
```

The envelope is a closed three-way union:

- `completed`: `ok: true`, `resultKind: "completed"`, empty `errors`, no
  `stopCode`, and tool-specific completed `data`;
- `stopped`: `ok: false`, `resultKind: "stopped"`, a required stable
  `stopCode`, a non-empty `errors` array whose primary code equals `stopCode`,
  and the exact evidence-bearing stop `data` from the matrix below;
- `rejected`: `ok: false`, `resultKind: "rejected"`, no `stopCode`, a non-empty
  `errors` array, and `data: TaskErrorData { code, context,
  allowedNextActions[] }`.

`resultKind` and its field invariants are the outer discriminator. A completed
classification/session/verification and its evidence-bearing stopped outcome
may intentionally use the same named domain-data schema, whose required outcome
fields distinguish the observation. A `rejected` result never borrows either
domain-data shape. Its primary `errors[0].code` equals `TaskErrorData.code`;
remaining errors are supporting diagnostics and never encode a second domain
outcome. `context` is a named redacted closed record selected by `code`.

A mutating request and response additionally contain the same `operationId`.
A read-only response omits `operationId` unless `data` reports a referenced
active/terminal operation. `data` is one of the tool-specific variants below;
`evidence` contains only bounded redacted identities, hashes, receipts, and
diagnostics. `command`, raw `stdout`, and raw `stderr` are always absent.

Stops/rejections after task lookup return their exact variant with the
unchanged/blocking/recovery task status. Schema/unknown-tool errors remain
application/MCP errors before a task result exists.

`branched.status` for a missing task is a successful read with
`status: "notCreated"` and `NotCreatedData { exists: false, startAllowed,
blockers[] }`. A failed `branched.start` preflight returns the `rejected` variant
with `status: "notCreated"`; its project-scoped start-attempt record preserves
replay without creating a task directory. Every other tool returns stable
`taskNotFound` when no task record exists.

Evidence-bearing domain stops are exhaustive:

| Producer | `stopCode` | Stopped `data` |
| --- | --- | --- |
| `delivery.verify` | `artifactKindMismatch` | `ArtifactVerificationData` with expected and observed classification |
| `merge.prepare(supportedUpdate)` | `twiceChangedProperties`, `unresolvedReferences` | `MergeSessionData` with `sessionId`, immutable/evolving digests, conflict count, and optional resolution-workspace ID; conflict records come from `merge.conflicts`, and current handles from `branched.status` |
| `merge.prepare(resolvedReplay)` | `conflictDecisionsIncomplete`, `unboundResolutionChanges` | `ResolutionReplayStopData { sessionId, baseSessionDigest, decisionSetDigest, workspaceGenerationId, missingConflictIds[], unboundChangeReceiptIds[] }`; missing decisions take primary-code precedence when both lists are non-empty |
| `merge.prepare` | `vendorAncestryMismatch` | `MergePreparationStopData { mode, expectedAncestor, observedAncestor, checkpointId, recovery: RecoveryPlanStatus }`; status is `recoveryRequired`, with task-checkpoint restore/recreate planned to `localVerified` |
| `merge.prepare(mainIntegration)` | `relevantBaselineChanged` | `MainPreparationStopData { comparisonId, expectedAnchor, observedAnchor, mismatchKinds: [relevantBaselineChanged], differenceManifestId, differenceDigest }`; no lock/original effect, downstream synchronization evidence is invalidated, and status is `localVerified` |
| `merge.prepare(mainIntegration)` | `mainPreparationMismatch` | `MainPreparationStopData { comparisonId, expectedAnchor, observedAnchor, mismatchKinds[], differenceManifestId, differenceDigest }`; status is `validationFailed` and no main session is published |
| `merge.verify(localCheckpoint)` / `merge.verify(mainSandbox)` | `validationFailed` | `MergeVerificationData` with immutable diagnostics evidence |
| `merge.verify(synchronizedTask)` | `validationFailed`, `unexpectedDelta` | `MergeVerificationData` with immutable diagnostics/difference evidence |
| `merge.verify(mainIntegration)` | `mainMergeValidationFailed` | `MainMergeValidationStopData { verification: MergeVerificationData, recovery: RecoveryPlanStatus }`; status is `recoveryRequired` |
| `branched.archive(outcome="abandoned")` preview from `mainMerged`/`mainValidated` | `abandonmentRecoveryRequired` | `AbandonmentRecoveryStopData { recovery: RecoveryPlanStatus }`; it persists preview evidence only, leaves status unchanged, and requires `repository.recover` before archive can be previewed again |
| `repository.lock` | `repositoryLockConflict` | `RepositoryLockConflictData` |
| `repository.lock` | `repositoryLockRollbackFailed` | `RepositoryLockRollbackFailedData` |
| `merge.apply(target="original")` pre-effect guard | `relevantBaselineChanged`, `additionalLocksRequired` | `MergeApplyStopData { sessionId, expectedAnchors, observedAnchors, lockSetId, expectedLockSetDigest, additionalLockEntries[], requiredNextTool: repository.unlock }`; status is respectively `staleRelevantBaseline` or `lockPlanExpansionRequired`, and the original is unchanged |
| `repository.commit` | `repositoryCommitFailed` | `RepositoryMutationObservationData { operationId, observedRepositoryAnchor?, observedObjects[], observedLocks[], recovery: RecoveryPlanStatus }`; status is `commitBlocked` |
| `repository.commit` | `repositoryCommitAmbiguous`, `repositoryUnlockUnverified` | `RepositoryMutationObservationData { operationId, observedRepositoryAnchor?, observedObjects[], observedLocks[], recovery: RecoveryPlanStatus }`; status is `committedUnverified` |
| `repository.unlock` | `repositoryUnlockUnverified` | `RepositoryMutationObservationData { operationId, observedRepositoryAnchor?, observedObjects[], observedLocks[], recovery: RecoveryPlanStatus }`; status is `recoveryRequired` |
| target-effect-free `readOnly` platform inspection with proven process-tree termination | `operationTimedOut` | `ReadOnlyTimeoutData { operationClass, observedTermination, temporaryEvidenceDiscarded: true, resumePhase }`; no operation ID exists and status is unchanged |
| `contained` operation with proven process termination and owned-area postcondition | `operationTimedOut` | `ContainedTimeoutData { operationId, operationClass, observedTermination, observedOwnedState, retainedEvidenceIds[], resumePhase }`; status remains the pre-operation safe phase |
| authoritative effect, or any effect whose postcondition is not proven | `operationTimedOut`, `operationEffectUnknown`, `rollbackUnproven` | `RecoveryRequiredData { recovery: RecoveryPlanStatus }`; status is `recoveryRequired` unless the more specific commit-ambiguity state applies |

Every stable code not listed in this matrix is a `rejected` result with its
named `TaskErrorData.context`; it cannot carry a tool-success/stop data variant.

## Execution Policies

Policy is selected from the request's closed discriminator before dispatch. A
tool descriptor may declare one default only when all variants share it;
otherwise it publishes an exhaustive variant-to-policy map, as
`repository.recover` does for apply and cancellation.

| Policy | Meaning |
| --- | --- |
| `readOnly` | No external or durable mutation; no `operationId` or `dryRun` |
| `localJournaled` | Requires `operationId`; journals only owned local state/work-root creation or an atomic task decision; no external repository/infobase effect and no fake `dryRun` |
| `contained` | Requires `operationId`; records the operation and mutates only an owned probe/sandbox/evidence area; external sandbox calls use intent/effect-unknown/observed/terminal barriers; no fake `dryRun` |
| `preparedJournaledEffect` | Requires `operationId` plus an exact prepared/session/status digest approval; journals intent before an authoritative infobase effect and verifies/reconciles its postcondition; no `dryRun` |
| `journaledEffect` | Requires `operationId` plus an exact guard digest; writes intent before external effect and verifies postcondition; no `dryRun` |
| `previewedJournaledEffect` | `dryRun: true` is target-effect-free, persists only its operation/preview evidence, and returns a preview-only data variant plus exact effect digest; a distinct applied operation binds that digest, journals intent, performs the owned/external effect, and verifies or reconciles postconditions |

Unknown-effect replay is never an execution policy. It returns
`operationEffectUnknown` until `repository.recover` reconciles the recorded
effect.

A preview and its apply are distinct requests with distinct operation IDs. The
apply binds the immutable preview digest; reusing the preview's operation ID
with `dryRun: false` is an input mismatch.
Every `previewedJournaledEffect` request is a strict tagged union: preview omits
`dryRun` or supplies the literal `true` and has no approval field; apply requires
the literal `dryRun: false` plus the tool-specific approved digest. The schemas
do not model a generic required boolean.

## Task Lifecycle Tools

### `unica.branched.start` — `localJournaled`

Request:

```text
taskId: TaskId (required)
operationId: OperationId (required)
cwd: workspace selector (required)
profile: non-empty local profile name (required)
taskSummary: non-empty immutable task summary (required)
```

Start validates config schema, secret availability (not values), exact
capability row, target identity, leases, state/work paths, cleanup/comment
policy, and pre-existing unresolved tasks before it creates the durable journal
and owned instance. It has no repository/infobase effect and no fabricated
preview. Before task creation it writes a project-scoped start-attempt record,
so failed preflight is replayable without a disposable directory. `data` is
`StartData { instanceId, projectId,
profile, originalInfobaseKind, repositoryTransport, capabilityRowId,
workRootLocator: OwnedTargetLocator, commitCommentPreview }`.

### `unica.branched.status` — `readOnly`

Request: no fields beyond common `cwd`/`taskId`. Existing-task `data` is
`TaskStatusData { exists: true, instanceId, phase,
taskWorkspaceId?, activeOperation?, pendingDecisions[], anchors[], ownedLocks[],
validationGates[], artifactHashes[], resumeHandles[], recentOperations[],
recovery?, archive?, cleanupEligibility }`.
`taskWorkspaceId` appears from successful deployment until cleanup and lets a
new client resume typed tools without learning a path. `resumeHandles` is a
closed tagged union containing only current non-invalidated records:

- `artifact { artifactId, role, kind, sha256, verificationId? }`;
- `workspace { taskWorkspaceId }` or `mergeResolutionWorkspace {
  sessionId, workspaceId, baseSessionDigest }`;
- `checkpoint { checkpointId, scope (local|synchronized), sourceFingerprint }`;
- `comparison { comparisonId, scope, leftAnchor, rightAnchor, deltaDigest }`;
- `mergeSession { sessionId, mode, checkpointId, incomingDistributionId?,
  comparisonId, baseSessionDigest, decisionSetDigest, resolvedSessionDigest?,
  conflictCount }`; `comparisonId` is the exact comparison produced/consumed by
  that session, including `mainIntegration`. `incomingDistributionId` is
  required for supported-update sessions (including resolved-replay results)
  and absent for main integration;
- `decision { decisionId, decisionKind, sessionId?, verificationId?,
  decisionDigest, revisedDecisionSetDigest? }`;
- `resolutionChangeReceipt { changeReceiptId, objectId, propertyPath,
  afterSha256, baseSessionDigest, workspaceGenerationId, receiptSequence,
  consumed }`; consumed or invalidated receipts are not selectable by
  `merge.resolve`;
- `verification { verificationId, scope, sessionId?, checkpointId?, outcome,
  verificationDigest, canonicalDeltaDigest, differenceManifestId?, differenceDigest?,
  adaptationDecisionId?, mergeReceiptId?, integrationSetDigest? }`; for
  `synchronizedTask+unexpected`, both difference fields are required, and other
  optional fields have the same exact scope/outcome presence rules as
  `MergeVerificationData`. `sessionId` is required for every scope except
  `localCheckpoint`; `checkpointId` is required for a valid local checkpoint or
  equivalent/adapted synchronized-task result and absent otherwise;
- `mergeApply { mergeReceiptId, target, sessionId, resolvedSessionDigest,
  resultFingerprint,
  rollbackCheckpointId?, sourcePublicationId?, sourceFingerprint?,
  taskInfobaseFingerprint?, integrationSetId?, integrationSetDigest? }`;
- `lockPlan { planId, planDigest, mergeSessionId, resolvedSessionDigest,
  verificationId, verificationDigest, integrationSetId,
  integrationSetDigest }`;
- `lockSet { lockSetId, lockSetDigest, planId, planDigest, integrationSetId,
  integrationSetDigest }`;
- `preview { toolName, previewOperationId, previewDigest, request }`, where
  `request` is the closed union `archive { outcome, reason? }`, `cleanup {
  archiveId }`, `deliveryCreate { role, inspectionDigest }`, `deliveryDeploy {
  distributionId }`, `repositoryUpdate { expectedStatusDigest }`, or
  `repositoryCommit { integrationSetId, expectedIntegrationSetDigest,
  lockSetId, expectedLockSetDigest, verificationId,
  expectedVerificationDigest }`;
- `recovery { priorOperationId, recoveryDigest }`;
- `archive { archiveId, sha256, outcome }`.

When present, `recovery` is the complete closed `RecoveryPlanStatus {
priorOperationId, target, effectClass, plannedResultPhase, observations[], actions[],
remainingUnknowns[], recoveryDigest }`, not only the compact resume handle.
`target` is `taskConfiguration`, `repositoryLocks`, `originalConfiguration`,
`repositoryCommit`, `artifact`, `archive`, or `cleanup`; `effectClass` is
`compensate`, `rollback`, `reconcileOnly`, `quarantine`, or `cleanup`. Each observation is a closed
`RecoveryObservation { observationKind, subjectId, expectedDigest?,
observedDigest?, outcome }`, where the kind is `repositoryAnchor`,
`objectFingerprint`, `taskFingerprint`, `lockOwnership`, `artifactPresence`,
`archivePresence`, or `quarantinePresence`, and outcome is `matches`, `differs`,
or `unknown`. Each action is a closed tagged union of `releaseOwnedLocks`,
`restoreOriginal`, `restoreTaskCheckpoint`, `recreateTaskInfobase`,
`verifyTaskFingerprint`, `observeCommit`, `quarantineArtifact`,
`resumeQuarantine`, `finishArchive`, or `finishCleanup`, with only registered
IDs/metadata object IDs, exact object or owned-role sets, immutable
checkpoint/session bindings, and expected postcondition digests required by
that variant;
it contains no command, credential, or path. `remainingUnknowns` contains typed
`{ observationKind, subjectId }` records. `recoveryDigest` covers the prior
operation, target/effect class, planned result phase, canonical observations,
exact ordered actions, remaining unknowns, and their anchors.
`repository.status.recovery` uses this
same type. Therefore a client that lost the original stop response can inspect
the exact predetermined recovery effects before approving the compact handle.
For an interrupted task deployment, task `merge.apply`, or compatible general
operation classified as an `authoritativeTaskConfigurationMutation`, recovery
may only restore the bound checkpoint or recreate the owned File IB and prove
its fingerprint; it never treats an unknown task mutation as successful or
blindly replays it.

At most one recovery plan is current. While it is current, read-only status is
allowed but every other mutating/authoritative call is rejected with
`recoveryPlanPending`; only the exact `repository.recover` apply may execute it.
A no-effect abandonment-preview plan may instead be cancelled by the exact
recover cancellation variant below. Cancellation atomically invalidates the
plan before normal main verification/commit can resume; an effect/recovery plan
that has entered `recoveryRequired` is never cancellable.

`recentOperations` contains bounded `{ operationId, toolName, terminalKind,
resultDigest }` records. Together the current phase and tagged handles provide
every ID/digest required by the next legal request after a response is lost;
callers never reconstruct IDs or paths. Status does not write observations or
reconcile a journal.

### `unica.branched.archive` — `previewedJournaledEffect`

Request fields: `taskId`, `operationId`, `outcome` (`success` or `abandoned`),
`reason` (required and non-empty for `abandoned`), and `dryRun?: true` for
preview; apply requires `dryRun: false` and `approvedPreviewDigest`. Success
requires `committedAndUnlocked`; abandonment requires no
worker, lock, original difference, or unknown effect. Preview `data` is
`ArchivePreviewData { outcome, retainedEntryNames[], excludedRoles[],
eligibilityDigest, previewDigest }`; applied `data` is `ArchiveData { archiveId,
outcome, schemaVersion, sha256, retainedEntryNames[], previewDigest }`.

An abandonment preview from `locked` is `rejected` with
`taskAbandonmentNotSafe` and
requires `repository.unlock(reason="abandonment")`; verified full release with
the original unchanged returns `synchronized`. From `mainMerged` or
`mainValidated`, the preview instead stops with
`abandonmentRecoveryRequired` and publishes a digest-bound plan that restores
the original rollback checkpoint, verifies the before fingerprints, releases
the complete lock set, and has `plannedResultPhase: synchronized`. The preview
performs no external effect and does not change phase. `repository.recover`
must execute that exact approved plan before a new abandonment preview can
become eligible. An active/unknown operation or absent rollback proof cannot
produce this plan and remains unsafe/recovery-bound.

### `unica.branched.cleanup` — `previewedJournaledEffect`

Request fields: `taskId`, `operationId`, `archiveId`, and `dryRun?: true` for
preview; apply requires `dryRun: false` and `approvedPreviewDigest`. Preview
`data` is `CleanupPreviewData { archiveId,
outcome, removableRoles[], ownedTargetLocator: OwnedTargetLocator, markerDigest,
previewDigest }`;
applied `data` is `CleanupData { quarantineId, outcome, removedRoles[],
retainedArchiveId, markerDigest, previewDigest }`. Preview and apply both rerun
every path/marker/reparse/Git/root guard. Apply writes intent, quarantines on the
same filesystem, and journals each role deletion so restart can reconcile a
missing, quarantined, partially removed, or fully removed owned instance.

## Delivery Tools

### `unica.delivery.inspect` — `readOnly`

Request: no fields beyond common `cwd`/`taskId`. `data` is `DeliveryInspectionData { configurationIdentity,
repositoryIdentity, bindingMatches, mainEqualsRepository,
mainEqualsDatabaseConfiguration, platformVersion, compatibilityMode,
deliveryPermissions { distributionAllowed, updateAllowed },
distributionRuleCounts, supportLayers[], localDifferences[], warningsAreErrors:
true, statusDigest }`. `configurationIdentity` is the exact
`ConfigurationIdentity` record and both delivery permissions are explicit
booleans. Platform warnings make inspection/create fail; there is no caller
switch to weaken this policy. Secret or raw connection fields are absent.

### `unica.delivery.create` — `previewedJournaledEffect`

Request fields: `taskId`, `operationId`, `role`
(`baselineDistribution` or `refreshDistribution`), `inspectionDigest`,
and `dryRun?: true` for preview; apply requires `dryRun: false` and
`approvedPreviewDigest`. A baseline apply records
`preflightPassed` only after revalidating that exact fresh clean inspection;
a refresh apply instead requires `localVerified`/later synchronization state,
or `blockedByForeignLock` with proven compensation, no owned lock/worker/unknown
effect, and a fresh clean inspection digest. In the latter case successful
creation atomically invalidates the old Dn, synchronization, main-session,
verification, plan, lock, and preview evidence and returns `localVerified`.
It does not claim the foreign lock disappeared; the later single bounded lock
attempt observes that fact without polling.
Source is always the proven clean original; ordinary `make`/`DumpCfg` and CFU
cannot satisfy the call. Preview `data` is `DistributionPreviewData { role,
configurationIdentity, repositoryAnchor, platformVersion, inspectionDigest,
plannedArtifactKind: configurationDistribution, previewDigest }`. Applied
`data` is `DistributionData { artifactId, role, kind, sha256,
configurationIdentity, repositoryAnchor, platformVersion, createdAt,
previewDigest }`. Preview never invents an artifact ID, output hash, or creation
time.

### `unica.delivery.verify` — `contained`

Request: `taskId`, `operationId`, `artifactId`, and optional `expectedKind:
AcceptedArtifactKind`.
Only a registered task artifact can be selected. `data` is
`ArtifactVerificationData { verificationId, artifactId, kind, expectedKind?,
expectationMatched, sha256, probeId, supportIdentity?, currentEqualsVendor?,
diagnosticsDigest }`. If `expectedKind` is present and differs from the observed
`kind`, the call stops with `artifactKindMismatch` while preserving this
classification evidence. With no expectation, classification itself completes
even for `configurationUpdate` or `invalidArtifact`; neither kind can satisfy a
later workflow input. A probe is destroyed only after its result is durably
observed; extension-only classification is forbidden.

### `unica.delivery.deploy` — `previewedJournaledEffect`

Request fields: `taskId`, `operationId`, verified `distributionId`, and
`dryRun?: true` for preview; apply requires `dryRun: false` and
`approvedPreviewDigest`. The artifact role must be
`baselineDistribution`; refresh distributions are merge inputs, not deployment
baselines. Preview `data` is `DeploymentPreviewData { distributionId,
distributionSha256, destinationKind: ownedTaskInstance, plannedRoles[],
previewDigest }`; applied `data` is `DeploymentData { taskInfobaseId,
taskWorkspaceId, distributionId, vendorIdentity, currentFingerprint,
vendorFingerprint, currentEqualsVendor, sourceFingerprint, previewDigest }`.
Preview never allocates or reports task-infobase/workspace IDs or post-deploy
fingerprints. The task IB is always local File; deployment also creates guarded
`v8project.yaml`, local overlay, and `.v8-project.json`.

## Merge Tools

### `unica.merge.compare` — `contained`

Request fields: `taskId`, `operationId`, `left`, `right`, and `scope`.
Each side is one of `originalCurrent`, `repository`, `taskCurrent`,
`taskVendor`, or `{ artifactId }`; `scope` is `projectDelta` or
`mainIntegration`. `data` is `ComparisonData { comparisonId, leftAnchor,
rightAnchor, platformReportId, canonicalManifestId, deltaDigest, changeCount,
unsupportedKinds[] }`. Related configurations map by UUID; name-only mapping
fails.

### `unica.merge.prepare` — `contained`

Request is a strict tagged union. `supportedUpdate` requires `checkpointId`,
verified `incomingDistributionId`, and project-delta `comparisonId`. A
replacement subvariant used only from `synchronizationConflicts` additionally
requires `replacesSessionId`, `expectedReplacedBaseSessionDigest`, and
`expectedReplacedDecisionSetDigest`; all three are absent for a first prepare.
Its checkpoint/distribution/comparison must exactly match the replaced
session's immutable inputs. It builds the fresh sandbox/session first, then
atomically invalidates the old resolution workspace, decisions, and all
unconsumed receipts; failure leaves the old session current.
`mainIntegration` requires the synchronized `checkpointId`, `verificationId`,
`expectedVerificationDigest`, and `expectedRepositoryStatusDigest`; it internally creates/registers and
classifies the ordinary result CF before comparing it with a repository-fresh
original snapshot. `data` is `MergeSessionData { sessionId, mode,
checkpointId, incomingDistributionId?, immutableInputHashes, anchorDigest,
settingsDigest, ordinaryResultArtifactId?, comparisonId, resultDigest?,
conflictCount, mergeResolutionWorkspaceId?, baseSessionDigest,
decisionSetDigest, resolvedSessionDigest? }`. `incomingDistributionId` follows
the same mode presence rule as the status handle.
`baseSessionDigest` never changes for that prepared workspace;
`decisionSetDigest` starts from the empty set and evolves after decisions;
`resolvedSessionDigest` exists only when all required decisions have been
materialized and zero conflicts remain. A resolution workspace ID appears only
for supported-update conflicts where manual/combine work is allowed.
`mainIntegration` must return `conflictCount: 0` and a resolved digest. A
relevant anchor change stops before session publication, invalidates D1-and-
later evidence, and returns to `localVerified` for a fresh distribution. A
merge conflict, unexpected scope, support mismatch, or extra repair stops with
`mainPreparationMismatch`, immutable comparison/difference evidence, and
`validationFailed` for task repair. Neither outcome enters the synchronization
conflict state. Every preparation restores a fresh sandbox; authoritative IBs
are unchanged.

`MainPreparationMismatchKind` is the closed enum
`relevantBaselineChanged`, `conflict`, `unexpectedScope`, `supportMismatch`, or
`extraRepair`. `MainPreparationStopData.mismatchKinds` is non-empty and contains
only that enum; the `relevantBaselineChanged` stop requires the singleton list
with that value, while `mainPreparationMismatch` excludes it.

`resolvedReplay` requires the prior supported-update `sessionId`,
`expectedBaseSessionDigest`, and `expectedDecisionSetDigest`. The server-owned
CAS journal supplies the complete canonical decision order; callers cannot
omit, duplicate, reorder, or inject IDs. Replay rejects incomplete conflict
coverage and every unconsumed/unbound resolution-workspace change. It then
restores the original checkpoint, replays that journal, and creates a new
immutable session. Zero remaining conflicts produces `resolvedSessionDigest`
and returns to `synchronizationPrepared`; otherwise the new session remains
`synchronizationConflicts`. This operation, not `merge.resolve`, owns sandbox
recreation and the conflict-state transition.

An incomplete/unbound replay attempt is a `stopped` observation, stays in
`synchronizationConflicts`, and performs no replay/sandbox recreation.
`ResolutionReplayStopData` returns both missing-conflict and unbound-receipt IDs;
`conflictDecisionsIncomplete` wins as the primary stop code while its list is
non-empty, then `unboundResolutionChanges` applies. The caller records exact
decisions or calls the digest-bound `supportedUpdate` replacement subvariant to
discard the entire old conflict workspace/session before a new operation.

### `unica.merge.conflicts` — `readOnly`

Request adds `sessionId`. `data` is `ConflictListData { sessionId,
baseSessionDigest, decisionSetDigest, mergeResolutionWorkspaceId?, conflicts[] }`.
Each conflict has `conflictId`, `objectId`,
display object, property path, `kind`, base/ours/theirs hashes, and allowed
resolutions. Kinds are `twiceChanged`, `deleteModify`,
`addAddNameCollision`, `uuidMismatch`, `unresolvedReference`,
`supportRuleBlocked`, or `mergeSettingsRejected`; resolutions are `takeOurs`,
`takeTheirs`, `combine`, or `manual` as allowed per kind.

### `unica.merge.resolve` — `localJournaled`

Request is a strict `decisionKind` union. `conflict` requires `sessionId`,
`conflictId`, `resolution`, non-empty `rationale`, and
`expectedBaseSessionDigest` plus `expectedDecisionSetDigest`.
`combine`/`manual` also require `changeReceiptId`, exact
`objectId`, `propertyPath`, and `expectedResultSha256`; the receipt must come
from the session's typed merge-resolution workspace and match its immutable
base-session digest. Its `data` is `ConflictDecisionData {
decisionId, conflictId, resolution, rationaleDigest, changeReceiptDigest?,
revisedDecisionSetDigest }`.

`adaptedDelta` requires an `unexpected` synchronized-task `verificationId`,
`expectedVerificationDigest`, `canonicalDeltaDigest`,
`differenceManifestId`, `differenceDigest`, and non-empty `rationale`. Its
`data` is `AdaptedDeltaDecisionData { decisionId, verificationId,
canonicalDeltaDigest, differenceDigest, rationaleDigest,
adaptationDecisionDigest }`. A subsequent `merge.verify` bound to that decision
must reproduce the same delta/difference before it may return `adapted` and
advance to `synchronized`. Only one decision can be recorded for an exact
verification/difference digest; a different second decision is rejected until a
new verification is produced. No authoritative IB is mutated by either
decision.

### `unica.merge.apply` — `preparedJournaledEffect`

Request fields: `taskId`, `operationId`, `sessionId`, `target` (`task` or
`original`), and `approval: DigestApproval` whose digest equals the exact
`resolvedSessionDigest`.
The `original` variant additionally requires `planId`, `expectedPlanDigest`,
`integrationSetId`, `expectedIntegrationSetDigest`,
`lockSetId`, and `expectedLockSetDigest`.
`data` is `MergeApplyData { mergeReceiptId, sessionId, resolvedSessionDigest,
target, beforeAnchor,
afterAnchor, resultFingerprint, supportAuditDigest, appliedDecisionIds[],
rollbackCheckpointId?, sourcePublicationId?, sourceFingerprint?,
taskInfobaseFingerprint?, integrationSetDigest?, lockSetDigest? }`.
Task apply replays update plus typed manual changes and proves D1 ancestry. It
then performs a full task-IB dump into staging, validates/builds that staged
source, atomically publishes it into the task workspace, proves canonical task
IB/XML fingerprint equality, and emits task-context cache/domain events before
success. The three publication/fingerprint fields are required for task apply
and absent for original apply; no compatible general dump/load call owns this
phase transition.
Original apply requires the exact integration/owned-lock sets and forbids
substantive repair in original. For the original variant,
`rollbackCheckpointId` is required and names the capability-proven recovery
source created and verified before mutation; it is absent for task apply.
Immediately before writing authoritative merge intent, the use case rechecks
relevant anchors and proves that the prepared change/reference closure is a
subset of the acquired lock set. A stale anchor or missing lock stops before any
original mutation, retains the exact owned locks, and requires
`repository.unlock(reason="rollback")`; successful unlock returns respectively
to `localVerified` (fresh Dn required) or `synchronized` (main preparation and
plan required). Intent is durable before either authoritative IB mutation;
unknown effect requires recovery rather than replay.

### `unica.merge.verify` — `contained`

Request is a strict scope union:

- `localCheckpoint` takes no scope-specific ID, freezes the current task
  IB/XML boundary, creates a checkpoint, and returns `valid|invalid`; successful
  data requires the new `checkpointId`;
- `synchronizedTask` requires `sessionId` and returns
  `equivalent|adapted|unexpected|invalid`; an adapted rerun additionally
  requires `adaptationDecisionId` and `expectedAdaptationDecisionDigest`.
  `equivalent|adapted` creates a new immutable synchronized checkpoint and
  requires `checkpointId` in success data; `unexpected|invalid` does not create
  one;
- `mainSandbox` requires the main-integration `sessionId` and
  `expectedResolvedSessionDigest` and returns `valid|invalid` before lock
  planning;
- `mainIntegration` requires `sessionId`, `expectedResolvedSessionDigest`,
  `mergeReceiptId`, `integrationSetId`, and `expectedIntegrationSetDigest` and
  returns `valid|invalid` after the original merge.

Omitting either adapted-decision field can produce only `equivalent`,
`unexpected`, or `invalid`. `data` is `MergeVerificationData { verificationId,
scope, outcome, canonicalDeltaDigest,
checkpointId?, validationReceiptIds[], supportAuditDigest,
selectedObjectFingerprints, differenceManifestId?, differenceDigest?,
adaptationDecisionId?, mergeReceiptId?, integrationSetDigest?,
verificationDigest }`. `adapted` requires a prior exact
adapted-delta decision and a reproduced difference; otherwise the first
non-equivalent result is `unexpected`. The use case runs configured checks
itself; caller prose or arbitrary receipt paths are rejected.

Every warning from a configured verification check is materialized as an
`invalid` outcome. Before original merge it therefore stops as
`validationFailed`; after original merge it stops as
`mainMergeValidationFailed` with the mandatory recovery plan. It never uses the
delivery-only `platformWarningRejected` code, so error precedence cannot bypass
diagnostic evidence or authoritative rollback.

An invalid `mainIntegration` observation does not roll back from this contained
verifier. It atomically enters `recoveryRequired` and publishes a
digest-bound `RecoveryPlanStatus` whose exact ordered actions restore the
original from the merge receipt's `rollbackCheckpointId`, verify the before
anchor/fingerprints, release the complete owned lock set, and then enter
`validationFailed`. Only `repository.recover` may execute that plan. Failure or
interruption keeps recovery required; direct transition to task repair is
forbidden until restoration and unlock are proven.

## Repository Tools

### `unica.repository.status` — `readOnly`

Request: no fields beyond common `cwd`/`taskId`. `data` is `RepositoryStatusData { bindingIdentity,
repositoryVersion?, originalInfobaseKind, repositoryTransport,
mainEqualsRepository, mainEqualsDatabaseConfiguration, journaledLocks[],
lastObservedConflicts[], conflictObservationCompleteness
(journalOnly|readOnlySnapshotProven),
conflictsObservedAt?, activeOperation?, recovery?, statusDigest }`. This is not a
promise of a global live-lock snapshot: without a separately proven read-only
capability it reports only journal/conflict evidence observed by prior calls.
Each `ObservedRepositoryConflict` requires `objectId`, object display,
`lockedBy`, `computer`, `infobase`, and `lockedAt`; the last four are explicit
typed values or `null`, never omitted or inferred.

### `unica.repository.update` — `previewedJournaledEffect`

Request fields: `taskId`, `operationId`, `expectedStatusDigest`, and
`dryRun?: true` for preview. Preview uses only read-only report/dump/compare evidence and
returns `RepositoryUpdatePreviewData { beforeAnchor, plannedChanges[],
plannedRelevantObjects[], plannedUnrelatedObjects[], structuralChanges[],
structuralConfirmationRequired, updateDigest }`. Each planned change has exact
metadata object ID, action (`add`, `modify`, or `delete`), and relevance reason.

A distinct apply operation requires `dryRun: false` and
`approvedUpdateDigest`, rechecks
the status/binding/clean anchors, journals intent, and returns
`RepositoryUpdateData { beforeAnchor, afterAnchor, changedRelevantObjects[],
changedUnrelatedObjects[], appliedStructuralChanges[], originalFingerprint,
updateReceiptId, updateDigest }`. When the approved plan contains exact
add/delete operations, the adapter may derive the platform's repository-update
structural confirmation; callers cannot request or widen it. That path requires
capability evidence. The call refuses stale plans, unowned local changes,
active task locks outside recovery, and any automatic database restructuring.

### `unica.repository.planLocks` — `contained`

Request fields: `taskId`, `operationId`, `comparisonId`, `mergeSessionId`,
`expectedResolvedSessionDigest`, `verificationId`, and
`expectedVerificationDigest` for a valid `mainSandbox` verification. `data` is
`LockPlanData { planId, mergeSessionId, resolvedSessionDigest, verificationId,
verificationDigest, integrationSetId,
integrationEntries[], integrationSetDigest, lockEntries[], relevantAnchors,
compatibilityMode, referenceClosureDigest, settingsDigest,
prevalidationDiagnosticsDigest, planDigest }`. Every integration entry contains
the exact metadata object ID/name, repository action (`add`, `modify`, or
`delete`), typed reasons, and required lock targets. Every lock entry names an
existing development object to acquire. Added objects remain in the integration
set even when only the configuration root/parent can be locked; broad
unexplained configuration locking fails.

### `unica.repository.lock` — `journaledEffect`

Request fields: `taskId`, `operationId`, `planId`, and `approval:
DigestApproval` for `planDigest`. `data` is `LockResultData { planId,
planDigest, integrationSetId, integrationSetDigest, lockSetId, acquired[],
relevantAnchors, lockSetDigest }` only when the complete set is owned.

A foreign lock returns the common `stopped` variant with
`RepositoryLockConflictData { failedObject, lockedBy: RepositoryOwnerIdentity | null, diagnostic,
requestedExternalAction, acquiredThenReleased[], compensationVerified: true,
relevantAnchors }` and phase `blockedByForeignLock`. Failed compensation instead
returns `RepositoryLockRollbackFailedData { failedObject, acquired[],
released[], retained[], retainedLockSetId?, retainedLockSetDigest?,
recovery: RecoveryPlanStatus }` and phase `recoveryRequired`; retained lock fields are required
together when non-empty. Partial/conflict data never uses the success variant.
Acquisition is per object with the bounded profile timeout and no polling; the
whole acquire/compensate operation also has the independently bounded
transaction deadline. No merge starts without exact final ownership proof.

### `unica.repository.unlock` — `journaledEffect`

Request fields: `taskId`, `operationId`, `lockSetId`, `expectedLockSetDigest`,
`reason` (`compensation`, `rollback`, or `abandonment`), and `approval:
DigestApproval` whose digest equals `expectedLockSetDigest` for the complete
currently owned set. Callers cannot select a smaller/broader subset. `data` is
`UnlockData { released[], retained[],
releaseVerified, originalRestored, unlockReceiptId }`. No force and no
unattributed/same-user pre-existing lock are allowed. From
`staleRelevantBaseline`, verified complete release plus proof that no original
merge occurred invalidates Dn-and-later evidence and returns `localVerified`.
From `lockPlanExpansionRequired`, the same proof invalidates main-session,
main-verification, plan, and lock evidence and returns `synchronized`. Any
retained/ambiguous lock or original difference enters `recoveryRequired` with
an exact recovery plan. `reason="abandonment"` is valid from `locked` only when
the original still equals its pre-merge anchor; verified full release returns
`synchronized`. After an original merge, unlock alone is rejected and the
archive-previewed restore-plus-unlock plan is required.

### `unica.repository.commit` — `previewedJournaledEffect`

Request fields: `taskId`, `operationId`, `integrationSetId`,
`expectedIntegrationSetDigest`, `lockSetId`, `expectedLockSetDigest`,
`verificationId`, `expectedVerificationDigest`, and `dryRun?: true` for
preview. Preview derives the immutable profile-rendered comment and returns
`CommitPreviewData { exactObjects[], comment, integrationSetDigest,
verificationDigest, lockSetDigest, commitDigest }`. The exact objects include
additions and deletions even when they have no separately acquired lock. A
distinct apply operation requires `dryRun: false` and
`approvedCommitDigest`; applied
`data` is `CommitData { commitReceiptId, repositoryVersion?, committedObjects[],
contentVerified, releasedObjects[], unlockVerified, repositoryAnchor }`.
Supplying a comment is rejected; changing task metadata/template after start or
a render that is empty/not task-bound returns `commitCommentPolicyMismatch`.
Release, no force, no reference clearing, and no `keepLocked` are invariants,
not input switches.

### `unica.repository.recover` — variant policy

Request is a strict decision union. `apply` requires `taskId`, `operationId`,
`expectedRecoveryDigest`, and `approval: DigestApproval` whose digest equals
`expectedRecoveryDigest` for the already-journaled or abandonment-previewed
recovery action. It uses `journaledEffect`; at effect intent it atomically enters
`recoveryRequired`, and only an observed terminal postcondition may publish
`plannedResultPhase`. `data` is `RecoveryData {
priorOperationId, observedExternalState, actions[], resultingPhase,
remainingUnknowns[], recoveryDigest }`. Despite its historical repository
namespace, it is the single reconciliation entry for recorded repository,
authoritative original/task-IB, artifact/archive, or owned-cleanup effects. It
may reconcile or finish only a predetermined compensation, rollback,
restore/recreate, quarantine, or cleanup action; it cannot choose merge
resolutions, widen lock scope, force unlock, replay an unknown task mutation, or
retry an unknown commit.

`cancelPendingPlan` requires `taskId`, `operationId`,
`expectedRecoveryDigest`, and `decision: "cancel"`, with no approval object. It
uses `localJournaled` and is legal only for a current no-effect
`abandonmentRecoveryRequired` preview while phase and all source anchors still
equal that plan. It returns `RecoveryPlanCancellationData { recoveryDigest,
resultingPhase }`, invalidates the plan atomically, performs no external effect,
and leaves the recorded `mainMerged` or `mainValidated` phase unchanged. A
stale, effect-started, or any other recovery plan is not cancellable.

## Exact Lock-Plan Mapping

| Canonical change | Required repository development objects |
| --- | --- |
| Modify top-level object or non-development child/property | Owning top-level development object |
| Modify existing form/template represented as development object | That form/template object |
| Add top-level metadata object | Configuration root; the new object does not exist to lock yet |
| Add subordinate development object | Parent development object owning the collection |
| Add attribute/tabular section/dimension/resource/non-development child | Owning development object |
| Delete development object | Deleted object, parent, subordinate development objects, and every changed referrer |
| Delete non-development child | Owning object plus changed reference-cleanup closure |
| Change configuration-root property or top-level ordering | Configuration root |
| Change serialized/reference property to a new object | Referencing development object |

Planner inputs are the platform comparison report, canonical UUID/property
delta, metadata ownership graph, reference graph, compatibility mode, generated
merge settings, and diagnostics from the prevalidated main sandbox. Unknown
ownership/reference kinds return `unsupportedChangeKind`; file count, name-only
mapping, or whole-configuration fallback is forbidden.

`integrationSetDigest` covers every canonical add/modify/delete entry, its
reasons, the prepared main-session/result digest, reference closure, and
required lock targets. It cannot cover the later authoritative merge receipt.
`lockSetDigest` covers only observed acquired locks. Original merge and commit
bind both digests, while the post-merge verification additionally binds the
authoritative merge receipt. Thus a newly added object cannot disappear merely
because it did not exist as an independently lockable repository object.

## Stable Error Contract

| Code | Trigger and required behavior |
| --- | --- |
| `repositoryBindingMismatch` | Original is not bound to expected repository; stop before mutation |
| `mainDiffersFromRepository` | Unowned local difference exists; stop before distribution/update |
| `artifactKindMismatch` | Verified artifact kind differs from explicit `expectedKind`; retain classification evidence and do not advance |
| `artifactNotDistribution` | Dn is ordinary/configuration-update/invalid; do not deploy or synchronize |
| `platformWarningRejected` | A warning occurred in a delivery inspection/create/verification/deploy boundary; accept no delivery artifact/result. Configured merge-verification warnings instead become `invalid` and use `validationFailed` or `mainMergeValidationFailed` according to scope |
| `vendorAncestryMismatch` | The selected IDs already passed input/digest checks, but the authoritative task vendor no longer matches its D0/Dn checkpoint; enter `recoveryRequired` with an exact task-checkpoint restore/recreate/fingerprint plan whose successful result is `localVerified` |
| `twiceChangedProperties` | Explicit conflicts remain; enter `synchronizationConflicts` |
| `unresolvedReferences` | Reviewed closure is incomplete; no implicit clearing |
| `unexpectedDelta` | Delta is missing/extra/unapproved; no lock acquisition |
| `adaptationDecisionAlreadyRecorded` | A different decision already binds the exact verification/difference; produce a fresh verification instead of overwriting audit history |
| `conflictDecisionsIncomplete` | Resolved replay lacks decisions for one or more exact conflict IDs; stay `synchronizationConflicts`, return all missing IDs, and perform no replay or sandbox recreation |
| `unboundResolutionChanges` | Resolved replay observed unconsumed or unbound changes in its exact resolution-workspace generation; remain `synchronizationConflicts`, bind valid changes through exact decisions or use the digest-bound `supportedUpdate` replacement subvariant to invalidate/recreate the whole conflict session; perform no replay |
| `validationFailed` | Local-checkpoint, synchronized-task, or pre-lock main-sandbox validation failed; create no checkpoint/verification success, retain immutable diagnostics, and remain in or enter `validationFailed` for repair in the task workspace |
| `repositoryLockConflict` | Foreign lock; compensate owned subset and request external release |
| `operationTimedOut` | Finite process deadline elapsed; after termination and observation, a proven-contained operation returns to its recorded safe phase, while an authoritative or unproven effect requires recovery; never auto-retry |
| `repositoryLockRollbackFailed` | Compensation is unverified; enter `recoveryRequired` |
| `repositoryUpdatePlanStale` | Status/binding/anchor no longer matches approved update digest; create a new preview |
| `repositoryStructureConfirmationUnproven` | Exact add/delete update requires an unproven platform confirmation path; perform no update |
| `relevantBaselineChanged` | Relevant anchor moved; during main preparation (no locks/effect) invalidate synchronization evidence and return `localVerified`; during the original pre-effect guard retain the owned lock set in `staleRelevantBaseline`, then exact full unlock returns `localVerified`; fresh Dn is required in both cases |
| `mainPreparationMismatch` | Main sandbox has a conflict, unexpected scope, support mismatch, or extra repair; publish immutable difference evidence, enter `validationFailed`, and create no main session/lock plan |
| `additionalLocksRequired` | Before original mutation, enter `lockPlanExpansionRequired` with the exact retained lock set; verified full unlock returns `synchronized` and invalidates main preparation/plan evidence |
| `mainMergeValidationFailed` | Post-original-merge validation failed; enter `recoveryRequired` with an exact rollback-plus-unlock plan, and reach `validationFailed` only after `repository.recover` proves restoration and release |
| `repositoryCommitFailed` | Enter `commitBlocked` with a reconcile-only exact plan before any unlock/retry decision |
| `repositoryCommitAmbiguous` | Enter `committedUnverified` with an exact observation plan; no retry/cleanup |
| `repositoryUnlockUnverified` | From commit enter `committedUnverified`; from standalone unlock enter `recoveryRequired`. Both publish an exact observation/compensation plan and allow no archive/cleanup until reconciled |
| `cleanupNotAllowed` | Terminal/archive proof is incomplete; retain data |
| `abandonmentRecoveryRequired` | Abandonment was requested after original merge; preview the exact rollback-checkpoint plus full-unlock plan, leave phase unchanged, and require approved `repository.recover` before archiving |
| `unsafeTaskPath` | Marker/path/root/reparse guard failed; perform no destructive action |
| `operationReplayMismatch` | Same operation ID has different canonical input; reject |
| `operationEffectUnknown` | External effect is unresolved; only recovery may proceed |
| `recoveryPlanPending` | One exact recovery plan is current; reject every mutation except its `repository.recover` apply, or the cancellation variant when it is a no-effect abandonment preview |
| `taskPhaseMismatch` | A lifecycle/merge/repository precondition, or a clean bridge phase with no safety blocker, does not allow the tool; perform no effect and return exact allowed phases |
| `approvalDigestMismatch` | Preview/session/plan/lock/recovery approval is missing, stale, or for another digest; perform no effect and request a fresh producer call |
| `changeReceiptStale` | Resolution workspace/session or ordinary descendant evidence invalidated the receipt; recreate/review instead of applying it |
| `taskMutationBlocked` | Exclusively for a compatible general mutation: worker, original difference, owned lock, unknown effect, or terminal phase is a safety blocker; this code wins over phase mismatch and requires rollback/recovery or a new task |
| `platformCapabilityUnproven` | Exact topology/platform row missing or stale; disable mutation |
| `supportLayerAmbiguous` | Exact vendor layer cannot be selected/round-tripped; no edit |
| `unsupportedChangeKind` | Ownership/delta/reference behavior is unimplemented; no approximation |
| `projectIdentityCollision` | Project ID maps to different canonical targets; refuse shared state |
| `stateRootRelocationRequired` | Project locator names a different durable root; block until explicit offline migration |
| `exclusiveRepositoryUserRequired` | Profile/account exclusivity is absent or contradicted; stop preflight |
| `rollbackUnproven` | Required restore/unlock postcondition cannot be proven; recovery required |
| `taskAbandonmentNotSafe` | Worker/lock/difference/unknown effect remains; no abandoned archive |
| `profileInvalid` | Local schema, topology, path, or inline-secret rule failed; create no task |
| `secretUnavailable` | A referenced secret is absent/empty; create no process or durable secret derivative |
| `stateCorrupt` | Durable schema/hash/permissions are invalid; perform no external effect |
| `operationInProgress` | A live recorded operation/lease owns the target; attach/status instead of spawning |
| `taskNotFound` | A non-start lifecycle tool references `notCreated`; call status/start first |
| `taskWorkspaceContextInvalid` | `branchedTask` ID, marker, project binding, or lease authenticity does not match; do not expose or use a path |
| `toolNotBranchedCompatible` | A general tool has not declared `supportsBranchedTask`; reject before dispatch |
| `commitCommentPolicyMismatch` | Frozen template/task metadata cannot produce the exact task-bound comment; do not commit |
| `integrationSetMismatch` | Plan, merge, verification, commit set, or lock set digests disagree; rollback/stop before commit |

Raw localized text is supporting redacted evidence, never the stable code.
Error selection is deterministic: context identity/authenticity failure returns
`taskWorkspaceContextInvalid`; after authentic task lookup a current recovery
plan returns `recoveryPlanPending` for every mutation except its exact recover
apply or permitted no-effect cancellation. Otherwise an authentic compatible
mutation with a safety blocker returns `taskMutationBlocked`; only then can an
otherwise clean wrong-phase call return `taskPhaseMismatch`.
