# ADR-0012: Safe branched development for 1C configuration repositories

- Status: accepted
- Date: 2026-07-21
- Issue: [#137](https://github.com/IngvarConsulting/unica/issues/137)
- Acceptance: [Branched development acceptance](../acceptance/branched-development.md)
- Tool contract: [Branched development tool contract](../contracts/branched-development-tools.md)

## Context

A configuration connected to a 1C configuration repository cannot safely use
ordinary XML load as its development boundary. A complete task cycle needs a
fresh repository baseline, isolated development, a three-way supported update,
an exact repository lock plan, bounded integration into the still-bound
original infobase, one repository commit, verified unlock, recovery, and safe
disposal of task artifacts.

Issue #137 proposes that workflow through 21 domain tools and a packaged
`unica:branched-development` skill. The direction is correct, but the current
Unica implementation cannot be treated as its backend:

- `WorkspaceStateRepository` and `UNICA_CACHE_DIR` are deliberately volatile;
- runtime jobs can become `Lost` and cannot attach to an unowned process;
- `WorkspacePathPolicy` protects writes inside one workspace, not destructive
  cleanup of an external task root;
- `unica.support.edit` changes matching rules across support layers and its
  capability switch resets all vendor/object flags;
- the pinned `v8-runner` 0.5.1 does not implement distribution creation,
  supported `/UpdateCfg`, or configuration-repository operations, and its merge
  path updates the database configuration automatically;
- Designer's documented batch interface does not provide a general read-only
  lock/owner listing, and official documentation does not promise atomic
  multi-object lock or commit behavior.

The workflow in issue #137 adapts Appendix 2 of v8std #709 by replacing a
separate technical-project repository with an owned disposable File IB and XML
workspace. It preserves the required synchronization-before-locking rule, but
it is an adaptation rather than literal compliance with the standard's separate
repository recommendation.

## Decision

### Product boundary and completion

Unica adds one bounded context named `branched-development`. It owns a complete
task against an existing main configuration already connected to a 1C
configuration repository:

```text
repository-clean original
  -> verified distribution D0
  -> disposable File IB + platform XML workspace
  -> local development and verification
  -> verified current distribution D1
  -> supported three-way update (L1, D0, D1)
  -> explicit conflict decisions and equivalent/adapted delta proof
  -> prevalidated main integration and exact lock plan
  -> compensated lock acquisition
  -> bounded merge into the still-bound original configuration
  -> maximum validation
  -> one repository commit with release
  -> verified archive and owned cleanup
```

The original infobase remains connected to the same repository for the entire
cycle and never receives XML sources. D0 and D1 are full distribution CF files
created through `/CreateDistributionFiles -cffile`; the final integration
artifact is an ordinary CF. CFU exists only as a verification classification so
it can be rejected; no workflow input accepts it.

The disposable task, probe, and merge infobases are local File IBs. The original
infobase remains in scope as either File or client/server, and the repository
transport may be file or server; each topology requires its own capability row.

The feature is not complete until modify, add, delete, form/attribute ownership,
reference closure, multi-vendor support isolation, interruption recovery, and
the full acceptance matrix are proven. A smaller vertical slice is an
implementation milestone, not completion of issue #137.

The packaged workflow also requires the general native `unica.code.patch` tool
tracked by [issue #73](https://github.com/IngvarConsulting/unica/issues/73).
It is a separately owned prerequisite rather than one of the 21 lifecycle
tools. Issue #137 is publishable only after that exact tool and its own contract
tests are present on the base branch or integrated here; direct shell/file
scripts are not an acceptable substitute for the prompt-visible full cycle.

Repository creation/administration, administrative unlock, extension
repositories, production database restructuring, separate technical-project
repositories, and replacing the 1C repository with Git remain out of scope.

### Public MCP surface

The only public server remains `unica`. The bounded context exposes the 21
domain operations accepted in issue #137:

| Group | Tools |
| --- | --- |
| Task lifecycle | `unica.branched.start`, `status`, `archive`, `cleanup` |
| Delivery | `unica.delivery.inspect`, `create`, `verify`, `deploy` |
| Merge | `unica.merge.compare`, `prepare`, `conflicts`, `resolve`, `apply`, `verify` |
| Repository | `unica.repository.status`, `update`, `planLocks`, `lock`, `unlock`, `commit`, `recover` |

The public layer describes developer operations, not Designer flags or
`v8-runner` commands. Schemas reject raw `args`, executable paths, command
fragments, repository passwords, CFU, raw `-force`, `-revised`,
`-ClearUnresolvedRefs`, and `-keepLocked` controls.

Every mutating call requires a caller-stable `taskId` and `operationId`.
Replaying a terminal operation with the same canonical input returns its
recorded result. Replaying a live operation returns/attaches to its recorded
status without spawning another effect. Replaying `effectUnknown` refuses the
effect and requires `repository.recover`. Reusing the ID with different input
fails `operationReplayMismatch`.
`operationId` is a canonical UUID string and is safe as an operation-record key.
Read-only calls do not manufacture operation IDs. Their response includes an
operation ID only when reporting an active or completed operation.

The existing `OperationResult` envelope gains typed workflow fields while
remaining source-compatible for existing tools. Every task-bound branched,
delivery, merge, and repository response contains `ok`, `resultKind`, `taskId`,
task `status`, `summary`, `warnings`, `errors`, an `evidence` object, and typed
domain `data` (an empty object when no payload is needed). A mutating response
also contains the required request `operationId`; a read-only response omits it
unless it is describing an active/completed operation. Existing `changes`,
`artifacts`, and `cache` remain present.

The result is a closed union. `completed` has `ok: true`, empty errors,
tool-success data, and no `stopCode`. An evidence-bearing domain `stopped`
result has `ok: false`, a required stable `stopCode`, a matching primary error,
and the exact stop-data variant. A precondition/safety `rejected` result has
`ok: false`, no `stopCode`, and only its typed `TaskErrorData`; it cannot borrow
success or stop evidence. Branched tools never populate public `command`, raw
`stdout`, or raw `stderr`; bounded redacted platform diagnostics belong in
`evidence`. Contract tests lock this presence/absence matrix.

Execution policy is per closed request variant (with a tool-level default only
when every variant agrees), not inherited blindly from the current global
`cwd`/`dryRun`/`confirm` set. The registry exposes the exact variant-to-policy
map. A meaningful preview defaults to `dryRun: true`.
Mutations with no honest preview use an immutable sandbox prepare/apply pair.
The public switches `transactional`, `releaseLocks`, and
`forceReferenceCleanup` are removed. The draft's `warningAsError` is also not a
switch: delivery and validation warnings fail their boundary unconditionally.
Compensated acquisition, release on commit, strict warnings, and no forced
reference cleanup are invariants.

The error mapping remains phase-safe: delivery warnings use
`platformWarningRejected`; configured merge-verification warnings become an
`invalid` result and use `validationFailed` before original mutation or
`mainMergeValidationFailed` plus its recovery plan afterwards.

The corrected start request is:

```json
{
  "cwd": "/absolute/path/to/project",
  "taskId": "TASK-142",
  "operationId": "2ff0c844-e4d2-4ec9-a999-7ef3cbd65ccd",
  "taskSummary": "Add warehouse reservation checks",
  "profile": "main"
}
```

`taskSummary` is immutable task metadata. The named profile supersedes the issue draft's separate
`originalConnection`, `repositoryProfile`, and public `cleanupPolicy` fields;
their coordinated non-secret values, secret-reference identifiers, and secret
availability flags are resolved once and hashed into task evidence. Resolved
secret bytes and hashes of those bytes are forbidden.

`unica.branched.status` is strictly read-only. It may inspect journal and live
state, but only `unica.repository.recover` may persist reconciliation after an
interrupted dangerous operation. It returns typed, non-invalidated resume
handles for current artifacts, workspaces, checkpoints, sessions,
comparisons, decisions, authoritative apply receipts, verifications, plans, lock
sets, previews, recovery, and archive. The exhaustive closed union lives in the
tool contract; losing a terminal MCP response never forces the caller to know a
task path or invent an ID.

### Configuration and secrets

`v8project.yaml` remains the source-layout boundary, and
`v8project.local.yaml` remains the pinned runner's local overlay. Repository
keys are not added to that external schema. Unica introduces a separate,
local-only `unica.local.yaml` contract with named branched-development profiles.
A global branched-development section selects one shared disposable task work
root. Each named profile selects:

- the original infobase connection from the effective v8project configuration;
- an exact 1C platform executable whose reported version must match a capability
  row;
- repository location and a dedicated integration username;
- references to infobase and repository secrets;
- validation, commit-comment, and cleanup policies.

The schema denies unknown fields and starts with this contract (environment
names are references, not secret values):

```yaml
schemaVersion: 1
branchedDevelopment:
  projectId: 9bd51fb1-c13f-475f-9fa7-f15578d67b3b
  workRoot: /absolute/task/work/root
  profiles:
    main:
      original:
        connectionSource: v8project
        user: { env: UNICA_MAIN_USER }
        password: { env: UNICA_MAIN_PASSWORD }
      platform:
        executable: /absolute/path/to/1cv8
      repository:
        transport: file
        location: /absolute/path/to/repository
        user: unica-integration
        password: { env: UNICA_REPOSITORY_PASSWORD }
        accountUsage: exclusive
        lockCallTimeoutSeconds: 60
        lockTransactionTimeoutSeconds: 900
      validationPolicy: full
      commitCommentTemplate: "{taskId}: {taskSummary}"
      cleanupPolicy: archiveThenQuarantineDelete
```

`workRoot` and explicit executable/repository filesystem locations are absolute.
After canonical resolution, `workRoot`, durable state, coordination root,
original workspace, a File original-IB directory, and a file-repository
directory are pairwise non-overlapping: none may equal, contain, or be contained
by another. Start applies the same symlink/reparse/Git/root guards before it
creates an instance, so cleanup can never traverse into an original IB or 1C
repository even under a malicious nested profile.
`connectionSource: v8project` resolves the effective primary plus local overlay;
it never rewrites either file and imports only endpoint/options. Embedded
user/password values in the effective connection are rejected for this
workflow; credentials use the profile's environment references. Optional
infobase user/password references are omitted when the endpoint authenticates
without them. The repository account mode has only the safe `exclusive` value.
The only accepted cleanup policy for this full-cycle contract is
`archiveThenQuarantineDelete`; `archiveOnly` is rejected because it has no
`cleanedSuccess`/`cleanedAbandoned` terminal proof. Archive and every terminal/
path guard remain mandatory before quarantine/deletion.

The commit template accepts only `{taskId}`, `{taskSummary}`, and `{projectId}`.
Its rendered value is frozen at start, must be non-empty, and is the only
comment accepted by `repository.commit`. Per-object lock calls have a profile
timeout of 1-300 seconds; the whole acquisition has a 1-1800 second profile
timeout no shorter than one call. Effective lock deadlines are the smaller of
profile values and the capability-row maxima. Every other Designer operation
uses its required 1-7200 second capability-row deadline. Timeout cannot be
disabled or supplied through a tool.

Connection and repository-location parsers reject embedded usernames,
passwords, URI authority credentials, control characters, and raw Designer
fragments. A secret reference name may be persisted; its resolved value or hash
may not.

Secret values are never accepted inline. The first implementation resolves
environment-variable references; later secret providers implement the same
`SecretResolver` port. Task state stores reference names and booleans, never
resolved values. Known secret bytes are scrubbed from service messages,
stdout/stderr tails, errors, archives, and process summaries before any durable
write. `/P` and `/ConfigurationRepositoryP` are typed secret arguments and are
never rendered into public evidence.

Designer requires some credentials on its process command line. Unica prevents
its own logs and state from exposing them, but cannot hide an OS process command
line from a sufficiently privileged local observer. Shared-host deployments
must use OS account isolation or another platform-supported credential
mechanism; this residual risk is reported by preflight.

### Durable state, task identity, and paths

Dangerous workflow state is separate from ADR-0003 volatile cache state. The
default state root is the host's per-user application-state directory and can
be overridden only by `UNICA_STATE_DIR`. It contains schema-versioned task,
operation, merge-decision, lock, and archive records. `.build/unica` and
`UNICA_CACHE_DIR` are never the authoritative recovery source.

A small owner-only coordination root is deliberately not affected by
`UNICA_STATE_DIR`. It is resolved from the OS account profile API and contains
project/target locators, unresolved-task pointers, start-attempt replay records,
and the Unix lease files. The first valid start atomically registers
`projectId`, canonical original/repository identities, and the authoritative
state-root identity. A later process must follow that locator. Changing the
override while any task or attempt record exists fails closed; moving durable
state is an explicit offline migration with no unresolved operation. Thus a
crash followed by a different override cannot hide an unfinished task.

Default locations are deterministic:

| Host | Default durable state root | Non-overridable coordination root |
| --- | --- | --- |
| Linux | `${XDG_STATE_HOME:-$HOME/.local/state}/unica/state` | `$HOME/.local/state/unica/coordination` |
| macOS | `$HOME/Library/Application Support/Unica/state` | `$HOME/Library/Application Support/Unica/coordination` |
| Windows | `%LOCALAPPDATA%\Unica\state` | current-user known folder `Unica\coordination` |

The platform facade resolves these roots; application/domain code does not read
host environment conventions directly.

State and coordination directories/files use owner-only permissions
(`0700`/`0600` on Unix and an equivalent current-user ACL on Windows).
`UNICA_STATE_DIR` and `workRoot` must be absolute, non-overlapping, and neither
may contain the other or the coordination root.

`projectId` is a stable UUID stored in `unica.local.yaml`; it replaces the
undefined/mutable workspace fingerprint as the durable lookup key. Relocating
the project with the same local config preserves recovery lookup. Copying one
project ID to a workspace whose normalized original/repository identities differ
fails `projectIdentityCollision` rather than sharing state.

Original-infobase identity is SHA-256 over canonical topology and endpoint
(normalized File path, or normalized server/Ref), with all credentials removed.
Repository identity is SHA-256 over canonical transport/location plus the live
binding identity when the platform exposes it. Full digests are used as keys;
display values are separately redacted. An OS named mutex on Windows or an
advisory lock under the non-overridable coordination root on Unix is keyed by
these target identities. The target locator is checked while that lease is
held, so two different `UNICA_STATE_DIR` overrides cannot concurrently or
sequentially bypass an unresolved task.

An external `taskId` is a display/tracker identifier and must match
`[A-Za-z0-9][A-Za-z0-9._-]{0,63}`. Paths use a generated UUID `instanceId`, not
the external task ID. A completed cycle is never reopened as a new baseline;
the next task receives a new task ID and instance ID.

Start records configuration/repository identity, platform/standard versions,
cleanup and commit-comment policy, plus optional Git branch/commit evidence.
Git evidence is diagnostic only and never becomes the 1C merge base.
Before task-directory creation, the coordination root records the canonical
start input/result by workspace digest, task ID, and operation ID. Failed
preflight therefore creates no disposable task but still has deterministic
replay and input-mismatch behavior.

Durable state and disposable data are separated:

```text
<state-root>/branched-development/<project-id>/<instance-id>/
  task.json
  operations/
  merge-decisions/
  evidence/
  archive/

<task-work-root>/<instance-id>/
  .unica-owned.json
  baseline/
  current/
  sources/
  sandboxes/
  checkpoints/
  artifacts/
  logs/
```

The task source directory is a real Unica workspace. Unica generates its
`v8project.yaml`, local overlay, and `.v8-project.json` guard. Its absolute path
is not passed through the skill. `delivery.deploy` returns a
`taskWorkspaceId`; a merge session with manual/combine work may additionally
return a `mergeResolutionWorkspaceId`. Compatible existing tools accept a typed
`branchedTask` context selecting exactly one of the task workspace or a
session/digest-bound resolution workspace, while their ordinary `cwd` continues
to identify the original project. The application resolves the ID to the
marker-owned path, checks task/session phase and leases, substitutes the
effective workspace, and sends cache events to that disposable context.

A mutating existing tool additionally requires a stable operation ID and writes
a durable change receipt. A task-workspace mutation outside `developing` is
allowed only before any unresolved original effect/lock; it atomically returns
the task to `developing` and invalidates every descendant checkpoint, artifact,
merge decision/session, verification, integration/lock plan, and preview digest.
A resolution-workspace mutation instead produces a receipt bound to the
immutable base-session digest and workspace generation; it cannot change task
sources or phase. Decisions separately compare-and-swap the evolving
decision-set digest, so one decision does not stale other receipts from that
generation. Mutations are rejected throughout
acquisition/original-merge/commit/recovery/terminal phases. Tools that have not
declared branched-task compatibility reject the context. Tagged general tools
declare compatible variants individually; package tests snapshot the exact
read/edit/build/load/diagnostics/test operations used by the full-cycle skill.

Every compatible general-tool response passes a recursive projection before it
crosses MCP. Across structured and free-text fields, absolute task-work-root,
state, and coordination paths are replaced by workspace-relative logical names
or registered artifact IDs and roles, and secret/path-bearing diagnostics are
redacted. If a closed result cannot preserve its meaning after that projection,
the operation is not branched-compatible. This keeps the path opaque in
`changes`, artifacts, cache data, diagnostics, errors, and success summaries,
not merely in the request `cwd`.

XML is authoritative during ordinary development. After a platform-supported
update the task IB becomes authoritative until a full staged dump is published
and fingerprinted back to XML.

`.unica-owned.json` is created exclusively and contains schema version,
project/instance IDs, a task-ID hash, canonical original-infobase and repository
identity digests, canonical work-root identity, and random nonce. Cleanup
matches all of them against the coordination locator and task journal, then
rechecks the marker and canonical containment immediately before a
same-filesystem quarantine rename and before deletion. It rejects filesystem
roots, home, Git repositories/worktrees, the original workspace, symlinks,
junctions/reparse points, marker mismatch, root mismatch, and traversal at any
depth. This policy is separate from ordinary workspace-write policy.

Journal writes use write-ahead stages `intent -> effectUnknown -> observed ->
terminal`, canonical input hashes, atomic replace, file sync, and parent-directory
sync. Schema upgrades are explicit and monotonic. A process crash at any remote
effect boundary yields reconciliation, never blind replay.

### Leases and managed operations

Only one branched task may mutate a canonical original infobase, regardless of
repository username. An original-target OS lease is keyed by repository identity
plus original-infobase identity. A separate repository-account lease and
persistent reservation are keyed by repository identity plus normalized
integration username, so two different originals cannot concurrently reuse the
same exclusive account. Task/resolution-workspace mutations have their own
instance/session leases. Coordination locators index both target and account
reservations. Persistent records remain authoritative after the OS releases a
dead process's lease, so a new integration cannot start while either reservation
points to an unresolved task; release requires a proven safe terminal state.

Long `operationId`-bound contained or authoritative Designer calls run in a
dedicated branched-operation worker, not as a new `RuntimeJobOperation`. The
worker owns the process tree after MCP disconnect, writes operation-scoped
`/Out` and `/DumpResult` evidence, and commits a terminal record. A new MCP
process observes that record or contacts the live worker over an owner-only Unix
socket or Windows named pipe. The control record binds protocol version,
instance/operation IDs, PID plus process-start nonce, endpoint, and a random
256-bit token stored with owner-only permissions. The IPC accepts only
authenticated `status` and policy-allowed `cancel`; it cannot submit a new
Designer operation or merge decision. A stale PID/token/nonce is rejected.
Worker death leaves an effect to observe: a proven-contained owned-area outcome
returns to its recorded safe phase, while an authoritative or unproven effect
requires recovery. Neither is blindly restarted.

A strictly read-only tool may launch a bounded ephemeral inspection process but
never creates a detached operation, durable mutation, or `operationId`. On MCP
disconnect/timeout it kills the owned process tree, discards temporary output,
and may be called again because it is target-effect-free; it cannot yield an
unknown authoritative effect.

Cancellation of a sandbox/read operation is bounded and safe. Cancellation of
a repository or original-infobase mutation records intent and enters recovery
until postconditions prove the outcome. Cancellation is never reported as
successful rollback.

Every Designer call has a finite deadline from the validated profile/capability
row. A per-object lock call never auto-polls a foreign lock and cannot exceed
the profile's 1-300 second bound. On timeout Unica terminates the owned process
tree, observes postconditions, compensates attributable effects when proven,
and otherwise enters `recoveryRequired`; it never waits or retries indefinitely.

### State machine and abandonment

The normal phases are:

```text
created -> preflightPassed -> baselineReady -> developing -> localVerified
-> synchronizationPrepared -> synchronized
-> integrationPlanned -> acquiringLocks -> locked -> mainMerged
-> mainValidated -> committing -> committedAndUnlocked
-> archivedSuccess -> cleanedSuccess
```

`synchronizationPrepared` branches to `synchronizationConflicts` only when typed
conflicts exist; resolved decisions recreate the sandbox and return to
`synchronizationPrepared` before apply.

Blocking/recovery phases are `blockedByForeignLock`,
`staleRelevantBaseline`, `lockPlanExpansionRequired`, `unexpectedDelta`,
`validationFailed`, `commitBlocked`, `recoveryRequired`, and
`committedUnverified`.
`synchronizationConflicts` is the single canonical conflict-state name.

An unfinished task can be deliberately abandoned through
`unica.branched.archive(outcome="abandoned", reason="superseded")`. It reaches
`archivedAbandoned` only after proving that no worker is active, the original
configuration equals repository content for every touched object, no task-owned
lock remains, and no external effect is unknown. Cleanup can then reach
`cleanedAbandoned`. `recoveryRequired` and `committedUnverified` can never be
archived or cleaned. This closes the lifecycle without weakening successful
commit proof.

This is a guarded cross-cutting edge only from `created`, `preflightPassed`,
`baselineReady`, `developing`, `localVerified`, `synchronizationPrepared`,
`synchronizationConflicts`, `synchronized`, `integrationPlanned`,
`blockedByForeignLock`, `unexpectedDelta`, or
`validationFailed`. States that may still own an original mutation or unknown
commit cannot use it; they must first follow their rollback/recovery exit to an
eligible state.

That prerequisite has an exact public path. From `locked`, where no original
merge occurred, `repository.unlock(reason="abandonment")` proves complete
release and returns to `synchronized`. From `mainMerged` or `mainValidated`, an
abandoned `branched.archive` preview performs no target effect but publishes a
digest-bound recovery plan for the recorded rollback checkpoint, before-state
proof, and complete lock release. Approved `repository.recover` executes that
plan and returns to `synchronized`; only a new archive preview can then approve
abandonment. Missing proof or an unknown effect cannot create this shortcut.

Blocking exits are normative:

| Phase | Only allowed progress |
| --- | --- |
| `blockedByForeignLock` | after proven compensation/no owned locks and external coordination, a fresh clean inspection plus applied refresh distribution invalidates Dn-and-later evidence and returns `localVerified`; the next plan performs one bounded lock attempt and does not assume/poll release. Alternatively a task mutation abandons integration evidence and returns `developing` |
| `synchronizationConflicts` | record every explicit decision, then `merge.prepare(mode="resolvedReplay")` recreates the checkpoint sandbox and returns to `synchronizationPrepared` only with zero remaining conflicts; accidental/unbindable workspace changes instead require a digest-bound `supportedUpdate` replacement that atomically invalidates the old workspace/receipts/decisions only after a fresh session is durable |
| `staleRelevantBaseline` | original is still unchanged; `repository.unlock(reason="rollback")` must release the exact retained set and return `localVerified`, then a fresh Dn, synchronization, and plan are required |
| `lockPlanExpansionRequired` | original is still unchanged; exact full unlock returns `synchronized`, invalidates main preparation/plan evidence, and requires a new main sandbox and expanded plan |
| `unexpectedDelta` | record a digest-bound adapted-delta decision through `merge.resolve`, rerun `merge.verify`, or return to `developing`; never lock while unresolved |
| `validationFailed` | no original difference or owned lock may exist on entry; repair in the task workspace, whose next compatible mutation returns atomically to `developing` |
| `commitBlocked` | read-only external-state inspection followed by `repository.recover`; no retry |
| `recoveryRequired` | inspect the digest-bound exact recovery plan, then use `repository.recover` or documented external manual recovery followed by reconciliation; failed main validation may reach `validationFailed` only after original restoration and unlock are proven |
| `committedUnverified` | read-only reconciliation; transition only to `committedAndUnlocked` when both content and unlock are proven |

This naming and the success/abandonment terminal split deliberately supersede
the issue draft's inconsistent `syncPrepared`, `syncConflictResolution`,
`syncConflicts`, `archived`, and `cleaned` names. Safe abandonment supersedes
success-only cleanup only for disposable task data; `cleanedAbandoned` is never
repository success or completion of issue #137.

For every eligible phase above, guarded abandonment is an alternative to the
normal exit in the table. It is not available from `acquiringLocks`, `locked`,
`mainMerged`, `mainValidated`, `staleRelevantBaseline`,
`lockPlanExpansionRequired`, `committing`, `commitBlocked`,
`recoveryRequired`, or `committedUnverified`.

Successful tools own phase transitions; the skill cannot advance state by
assertion:

| Proven operation/postcondition | Resulting phase |
| --- | --- |
| `branched.start` local/profile/capability checks | `created` |
| baseline `delivery.create` consumes a fresh clean binding/repository inspection digest and records its pre-effect guard | `preflightPassed` |
| verified D0 deployment and guarded task workspace | `baselineReady`, then `developing` |
| `merge.verify(scope="localCheckpoint")` | `localVerified` |
| supported-update/resolved-replay `merge.prepare` | `synchronizationPrepared` or `synchronizationConflicts` |
| task `merge.apply` plus `merge.verify(scope="synchronizedTask")`, with a recorded digest-bound adaptation decision when needed and a new immutable synchronized checkpoint | `synchronized` |
| `merge.verify(scope="mainSandbox")` plus approved `repository.planLocks` | `integrationPlanned` |
| `repository.lock` journal/postcondition | `acquiringLocks`, then `locked` |
| original `merge.apply` | `mainMerged` |
| `merge.verify(scope="mainIntegration")` | `mainValidated` |
| `repository.commit` intent/content/unlock proof | `committing`, then `committedAndUnlocked` |
| `branched.archive` and `branched.cleanup` | matching success/abandoned terminal phases |

`merge.verify` accepts only the listed verification scopes. It executes the
configured validators itself and stores immutable receipts; prose or paths to
caller-created “passed” files cannot advance the phase.

### Platform and support boundaries

Application use cases depend on platform-neutral ports. OS-specific Designer
location, process creation, encoding, service-message capture, filesystem
durability, and reparse-point handling live under
`infrastructure/platform/**` as required by ADR-0009.

A typed `DesignerPort` models only approved operations: distribution creation,
ordinary CF creation, artifact probing, compare, supported update, merge,
configuration checks, repository update/dump/report, per-object lock/unlock, and
single-set commit. Each request separates public, path, and secret arguments.
No public tool can supply argv.

An unsuccessful capability spike cannot be bypassed by reading undocumented
repository storage or editing repository files directly. Unsupported Designer
behavior remains disabled. Domain/cache events are emitted only after an
observed successful mutation and target the original or disposable workspace
that actually changed.

Repository synchronization is previewed from read-only repository
report/compare evidence. The preview names every incoming modify/add/delete and
produces an immutable update digest. Apply revalidates a clean original and that
digest before mutation. When, and only when, the approved plan contains added or
deleted repository objects, the adapter may supply the platform's
`/ConfigurationRepositoryUpdateCfg -force` structural-change confirmation.
This is a derived adapter detail required to receive those exact objects, not a
public flag or a merge/conflict policy. It requires its own capability fixture
and cannot be reused for supported `/UpdateCfg`, merge, commit, unlock,
reference clearing, or administrative ownership override.

The pinned `v8-runner` remains available for its current build, dump, make,
test, syntax, and launch use cases. It is not the repository-cycle backend.
Repository operations and `/UpdateCfg` cannot route through
`unica.runtime.execute`.

The existing support editor is not used for this workflow until it is replaced
by the versioned layer-aware `unica.support.edit` contract. Branched calls use
`mode="layer"`, exact `layerId`, and either `setCapability` or
`setObjectState(objectId, state)`; the legacy broad `Capability`/UUID-all-layer
form is rejected in this workflow. The operation preserves every unrelated
byte/rule and proves the round trip. Ambiguous layer identity or unsupported
serialization fails `supportLayerAmbiguous`.

Objects are locked by default. An unchanged object temporarily made editable is
returned to locked before synchronization. After D1 update, a new vendor object
is non-editable and every existing object's prior support mode is preserved.
`offSupport` is outside ordinary branched development and the branched-task
schema rejects it outright. Any future release workflow for that transition
requires its own ADR and public approval contract. A source already supported
by its own distribution identity is rejected; a different upstream vendor
chain is preserved.

### Merge decisions and authoritative replay

`unica.merge.prepare` always starts from an immutable task checkpoint or a
repository-fresh original snapshot. Supported update runs `/UpdateCfg` without
`-force` in a disposable File IB and parses twice-changed properties into typed
conflicts.

`unica.merge.resolve` records either a typed conflict decision or a
verification/delta-digest-bound adapted-delta decision plus rationale; it does not
mutate an authoritative infobase. `takeOurs` and `takeTheirs` become explicit
merge settings. `combine` and `manual` reference a typed, task-owned,
fingerprinted change payload, never an arbitrary path.

A conflict session separates its immutable base-session digest from the
evolving decision-set digest. All resolution-workspace receipts bind the base
and workspace generation, so recording the first decision cannot invalidate a
second prepared manual receipt. Each decision compare-and-swaps the decision-set
digest; the server journal owns canonical order. Resolved replay binds both
digests, rejects incomplete decisions or unbound workspace changes, replays the
complete journal without a caller-supplied order, then destroys the old
resolution workspace.

Every settings change recreates the sandbox. The approved update is then
materialized in this order:

1. restore the clean checkpoint;
2. execute `/UpdateCfg` with generated settings;
3. confirm D1 is the resulting vendor ancestor;
4. dump XML to staging;
5. apply typed manual/combine changes in the sandbox workspace;
6. rebuild and validate the sandbox;
7. compute the canonical delta and result fingerprints.

`unica.merge.apply` repeats the same recorded steps against the authoritative
task IB, then owns a full staged XML dump, staged build/validation, atomic source
publication, task-context cache/events, and canonical task-IB/XML fingerprint
equality before success. A separate compatible general dump/load is not inserted
into this phase because its ordinary mutation invalidation would stale the
approved session. Both task and original targets use
intent/effect-unknown/observation/terminal journaling; a disconnect or timeout
cannot blindly repeat either authoritative mutation. This makes manual
decisions reproducible without losing vendor ancestry or leaving stale XML.

The disposable task IB is still authoritative during deployment and supported-
update replay. An interrupted task mutation gets a digest-bound recovery plan
whose target is `taskConfiguration`: it observes the task fingerprint, restores
the bound immutable checkpoint or recreates the owned File IB, and proves the
expected safe fingerprint/phase. Recovery never assumes the unknown mutation
succeeded and never blindly replays it.

Main-integration preparation clones a repository-fresh original configuration
into a non-repository File IB, including its support chain, and applies only the
canonical delta plus reviewed reference closure. The original is changed only
after the sandbox result, support isolation, lock plan, and relevant anchors are
verified.

Immediately before original merge intent, relevant anchors and the complete
change/reference closure are checked again against the acquired set. A stale
anchor enters `staleRelevantBaseline`; a missing lock enters
`lockPlanExpansionRequired`. Both stop before original mutation and retain the
exact owned lock set until `repository.unlock(reason="rollback")` proves full
release. The former returns to `localVerified` for a fresh Dn; the latter
returns to `synchronized` and invalidates main-preparation/plan evidence.

Before original mutation, `merge.apply` persists a capability-proven rollback
checkpoint in its receipt. If post-merge `mainIntegration` verification is
invalid, the contained verifier does not perform rollback. It atomically enters
`recoveryRequired` with an exact plan to restore the checkpoint, verify the
before fingerprints, release the complete owned lock set, and only then enter
`validationFailed`. `repository.recover` owns those authoritative effects; any
unknown restore or unlock remains recovery-required.

Only supported-update sessions may enter `synchronizationConflicts` and use
resolved replay. Main-integration preparation must produce zero conflicts; a
stale relevant anchor stops without locks/effects, invalidates D1-and-later
evidence, and returns to `localVerified`. An unexpected scope, conflict, support
mismatch, or extra repair stops as `mainPreparationMismatch` in
`validationFailed` with immutable comparison/difference evidence and no main
session. Neither path enters a conflict state with no legal exit.

All feature, integration, unit, and data-migration tests finish in the
synchronized task IB before locks. The locked window contains only bounded
configuration/UUID/reference/delta/diagnostics/ownership checks. Unica never
runs `/UpdateDBCfg`, database restructuring, or destructive runtime tests
against the original infobase as part of this workflow.

### Lock, rollback, commit, and recovery

Until a proven read-only ownership API exists, every profile must declare a
repository account dedicated exclusively to Unica integration. Unica serializes
that account and refuses a task while its durable state contains an unresolved
lock journal. Out-of-band use of the dedicated account violates the profile
contract and forces manual recovery when observed.

This mandatory exclusive-account rule and the absence of a forced-unlock API
deliberately supersede the issue draft's conditional same-user proposal and
ambiguous forced-recovery wording. They remain until a stronger ownership API
has its own capability proof.

Locks are acquired one development object per Designer call in deterministic
order. Intent is durable before the call and success is recorded immediately.
On failure, only objects whose acquisition belongs to the current operation are
unlocked in reverse order. An ambiguous same-user/pre-existing lock is never
released automatically. There is no automatic wait/poll loop. `lockedBy`
remains nullable unless a capability fixture proves localized owner extraction.

Rollback of main changes is not assumed. Before automatic integration is
enabled for a platform capability row, a real fixture must prove repository
restore and unlock without `-force` for modify, add, delete, reference change,
process interruption, and failed commit. Missing proof causes
`platformCapabilityUnproven`; ambiguous postconditions cause
`recoveryRequired`. There is no public forced-unlock escape hatch. External
manual recovery is reconciled read-only and then recorded by
`unica.repository.recover`.

Commit uses the exact verified integration set once, including additions and
deletions that have no independently acquired lock, with the frozen
policy-rendered task comment, without `-keepLocked` and without `-force`.
Success requires repository content equality and proof that every acquired lock
is released. Partial or ambiguous results are not retried and cannot be
archived as successful.

### Capability gating

Unica ships
`plugins/unica/references/branched-development/platform-capabilities.json`.
Each row is keyed by host OS/architecture, exact 1C platform version,
locale/encoding, original-infobase kind (`file` or `clientServer`), and
repository transport (`file` or `server`). Evidence lives as a redacted tracked
summary under `tests/fixtures/branched_development/platform_evidence/<id>.json`.

A row contains schema/feature-contract versions, harness contract digest,
implementation commit, run timestamp, topology key, all required case IDs,
per-case result/postcondition hashes, evidence path/SHA-256, and an overall
pass. CI recomputes the digest over the capability-critical adapter/domain/test
sources, verifies the evidence hash and exact case set, and rejects a row after
any relevant contract change. A platform/OS/topology claim with no exact valid
row fails preflight.

```json
{
  "schemaVersion": 1,
  "featureContractVersion": 1,
  "contractDigest": "0000000000000000000000000000000000000000000000000000000000000000",
  "rows": [{
    "id": "darwin-arm64-8.3.27.2074-en-file-file",
    "host": {"os": "darwin", "arch": "arm64"},
    "platformVersion": "8.3.27.2074",
    "locale": "en",
    "encoding": "utf-8",
    "originalInfobaseKind": "file",
    "repositoryTransport": "file",
    "timeoutsSeconds": {
      "distributionCreate": 1800,
      "ordinaryConfigurationCreate": 1800,
      "artifactProbe": 600,
      "taskDeploy": 1800,
      "sourceDump": 1800,
      "taskBuildLoad": 1800,
      "taskTest": 3600,
      "compare": 900,
      "supportedUpdate": 1800,
      "merge": 1800,
      "configurationCheck": 1800,
      "repositoryInspect": 600,
      "repositoryUpdate": 1800,
      "repositoryLockCallMax": 300,
      "repositoryLockTransactionMax": 1800,
      "repositoryUnlock": 600,
      "repositoryCommit": 1800,
      "rollback": 1800
    },
    "implementationCommit": "0000000000000000000000000000000000000000",
    "harnessDigest": "1111111111111111111111111111111111111111111111111111111111111111",
    "passedAt": "2026-07-21T00:00:00Z",
    "caseIds": ["distribution-kind", "partial-lock", "same-user-lock",
      "commit-atomicity", "supported-update", "support-isolation",
      "rollback", "multi-vendor-support", "manual-replay",
      "repository-structural-update"],
    "evidence": {"path": "darwin-arm64-8.3.27.2074-en-file-file.json",
      "sha256": "2222222222222222222222222222222222222222222222222222222222222222"},
    "passed": true
  }]
}
```

The timeout mapping is exact:

| Platform operation | Timeout key |
| --- | --- |
| Distribution CF creation | `distributionCreate` |
| Ordinary result CF creation | `ordinaryConfigurationCreate` |
| Artifact classification/probe IB | `artifactProbe` |
| Task File IB provision and baseline distribution load | `taskDeploy` |
| Task IB to XML staged dump | `sourceDump` |
| Compatible task configuration build/load/syntax operation | `taskBuildLoad` |
| Compatible configured platform test run | `taskTest` |
| `/CompareCfg` and canonical comparison capture | `compare` |
| Supported `/UpdateCfg` sandbox or task replay | `supportedUpdate` |
| Main sandbox clone/merge and original merge | `merge` |
| Platform configuration validation | `configurationCheck` |
| Repository status/history/dump/report | `repositoryInspect` |
| Repository configuration update | `repositoryUpdate` |
| One repository lock call | `repositoryLockCallMax` |
| Whole acquire/compensate sequence | `repositoryLockTransactionMax` |
| Repository unlock | `repositoryUnlock` |
| Repository commit | `repositoryCommit` |
| Repository restore/original rollback | `rollback` |

For a compatible general tool that already accepts a timeout, the effective
deadline is the lower of its validated request value and the mapped capability
deadline; omission uses the capability deadline. No Designer or platform-test
subprocess in this workflow falls through to an unmapped default.

A row is publishable only after a disposable real-platform fixture proves:

1. distribution classification and vendor-support creation;
2. partial lock diagnostics, owner availability, and compensation;
3. same-user pre-existing lock behavior;
4. commit failure/atomicity and unlock behavior;
5. supported-update settings for scalar/module/add-add/delete-modify/reference
   and vendor-rule conflicts without `-force`;
6. final merge support isolation for unsupported and upstream-supported targets;
7. rollback/restore/unlock for modify, add, delete, interruption, and disconnect;
8. layer-aware multi-vendor support editing and round trip;
9. reproducible manual conflict materialization and authoritative replay;
10. previewed repository update with incoming add/delete and exact internal
    structural-change confirmation.

Every row contains all timeout keys shown above. Missing/zero/out-of-range keys
invalidate the row. Evidence includes forced deadline termination and
postcondition/recovery results for repository update, one lock call, the whole
lock transaction, commit, rollback, and one sandbox operation; documentation or
a process exit alone does not prove safe timeout handling.

Items 7-10 are deliberate corrections added by design review: the original six
spikes did not close rollback, multi-vendor support, manual materialization, or
repository structural-update confirmation.
Task, probe, and merge sandboxes are always local File IBs. The original
infobase scenarios run against both File and disposable client/server IBs, and
repository scenarios run against every claimed file/server transport. Evidence
from one topology cannot enable another.

No matching capability row means preflight fails closed. Passing fake tests or
detecting a compatible executable version is not sufficient evidence.

### Testing and package contract

Pure domain tests cover state transitions, journal barriers, idempotency,
cleanup paths, secret redaction, canonical delta, ownership/reference closure,
merge settings, lock planning, compensation, and recovery at every dangerous
boundary. Fake Designer integration tests cover localized output, partial
effects, process death, relevant/unrelated repository advancement, rollback,
commit ambiguity, and unlock failure.

The real fixture uses only disposable repositories, File task/sandbox IBs, and
explicitly provisioned disposable File or client/server original IBs under an
owned test profile. It is opt-in, serialized, and required for every capability
row claimed by a release. Published-package E2E invokes only public MCP tools
and the packaged `unica:branched-development` skill.

The new skill is product-owned. `skill-upstreams.json` and its validator gain
`sourceKind: "product"`, `ownerRepository`, and `designPath`; product entries do
not fabricate upstream `trackingRef` or baseline commits. Official
platform/standard links remain references, not donor provenance, and the skill
must not be attributed to `v8-runner`. Package tests cover both provenance
variants, add the skill scenario, its separately required `unica.code.patch`,
all 21 lifecycle tools, ordered transcript guards, and generated-package smoke
while preserving the single server named `unica`.

## Verification

The normative requirement/evidence matrix is maintained in
`spec/acceptance/branched-development.md`. Issue #137 may be closed only when
every row is proven by current code, tests, real-platform evidence, and packaged
MCP behavior. Unsupported change kinds fail `unsupportedChangeKind`; they are
not approximated by XML-file counts or name-only mapping.

## Consequences

- Unica gains a durable operational-state class distinct from volatile cache.
- The application result and schema registry gain typed workflow extensions and
  per-closed-request-variant execution policies without adding a public server.
- A native Designer adapter and layer-aware support model become required even
  though existing v8-runner workflows remain supported.
- Dangerous platform behavior is capability-gated by recorded real evidence,
  not inferred from documentation or a version string.
- Full-cycle automation can stop safely for external coordination or recovery;
  it never converts uncertainty into a successful commit or cleanup claim.
- The implementation is intentionally multi-layered, but partial layers cannot
  redefine the completion boundary of issue #137.

## References

- [v8std #709](https://v8std.ru/std/709/)
- [1C distribution commands](https://kb.1ci.com/1C_Enterprise_Platform/Guides/Administrator_Guides/1C_Enterprise_8.3.27_Administrator_Guide/Appendix_7._Startup_command-line_options_of_1C_Enterprise/7.4._Running_Designer_in_batch_mode/7.4.7._Commands_for_creating_distribution_and_update_files/?language=en)
- [1C supported configuration update](https://kb.1ci.com/1C_Enterprise_Platform/Guides/Administrator_Guides/1C_Enterprise_8.3.27_Administrator_Guide/Appendix_7._Startup_command-line_options_of_1C_Enterprise/7.4._Running_Designer_in_batch_mode/7.4.6._Configuration_support/?language=en)
- [1C compare and merge commands](https://kb.1ci.com/1C_Enterprise_Platform/Guides/Administrator_Guides/1C_Enterprise_8.3.27_Administrator_Guide/Appendix_7._Startup_command-line_options_of_1C_Enterprise/7.4._Running_Designer_in_batch_mode/7.4.4._Configurations_and_extensions/?language=en)
- [1C configuration repository commands](https://kb.1ci.com/1C_Enterprise_Platform/Guides/Administrator_Guides/1C_Enterprise_8.3.27_Administrator_Guide/Appendix_7._Startup_command-line_options_of_1C_Enterprise/7.4._Running_Designer_in_batch_mode/7.4.15._Configuration_repository_operations/?language=en)
