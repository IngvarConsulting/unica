# ADR-0012: Safe branched development for 1C configuration repositories

- Status: accepted
- Implementation gate: confirmed by the owner on 2026-07-21
- Date: 2026-07-21
- Issue: [#137](https://github.com/IngvarConsulting/unica/issues/137)
- Acceptance: [Branched development acceptance](../acceptance/branched-development.md)
- Tool contract: [Branched development tool contract](../contracts/branched-development-tools.md)

## Context

A configuration connected to a 1C configuration repository cannot safely use
ordinary XML load as its development boundary. A complete task cycle needs a
fresh repository baseline, isolated development, a three-way supported update,
an exact repository lock plan, bounded integration into the still-bound
original infobase, one task-content repository commit, verified unlock,
recovery, and safe disposal of task artifacts. A supported target may first
require a separately audited human root-only repository version that changes
only the exact support rules needed for integration.

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
  -> support preflight in a repository-fresh main sandbox
  -> optional human root-only support prerequisite
  -> fresh Dn and supported rebase after that prerequisite
  -> prevalidated main integration and exact lock plan
  -> compensated lock acquisition
  -> bounded merge into the still-bound original configuration
  -> maximum validation
  -> one task-content repository commit with release
  -> verified archive and owned cleanup
```

The original infobase remains connected to the same repository for the entire
cycle and never receives XML sources. D0 and D1 are full distribution CF files
created through `/CreateDistributionFiles -cffile`; the final integration
artifact is an ordinary CF. CFU is out of scope; the artifact verifier may only
recognize it in order to reject it. No workflow input, support preflight, or
candidate-set producer accepts it.

The disposable task, probe, and merge infobases are local File IBs. The original
infobase remains in scope as either File or client/server, and the repository
transport may be file or server; each topology requires its own capability row.

The feature is not complete until modify, add, delete, form/attribute ownership,
reference closure, multi-vendor support isolation, interruption recovery, and
the full acceptance matrix are proven. A smaller vertical slice is an
implementation milestone, not completion of issue #137.

BSL authoring is a package capability, not a global prerequisite for this
lifecycle. The native `unica.code.patch` proposed by
[issue #73](https://github.com/IngvarConsulting/unica/issues/73) is one intended
writer, but issue #137 does not depend on that exact tool and can ship without
it. Any general writer used inside a task or manual/combine resolution workspace
must declare the exact `supportsBranchedTask` variant and return the durable
target/hash/event receipt required by this decision. If no compatible BSL
writer is advertised, only a task or conflict requiring BSL text mutation is
unavailable; metadata/form/other typed development and the repository lifecycle
remain valid. Direct shell/file scripts are never a fallback for the
prompt-visible workflow.

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

Every effecting mutating call requires a caller-stable `taskId` and
`operationId`. The strictly read-only
`repository.update(mode="supportPrerequisiteArm")` preview is the explicit
exception: it accepts neither `operationId` nor `dryRun`, creates no durable
preview handle, and is repeated after response loss. Its separate
`localJournaled` apply requires a stable operation ID plus
`approvedArmingDigest`.
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
map. An ordinary meaningful preview defaults to `dryRun: true`; the arm preview
above is a closed `readOnly` variant and has no `dryRun` field. Its apply is
`localJournaled`.
A missing honest preview does not imply a sandbox. The closed variant selects
its declared policy: `localJournaled` records only owned local-state/work-root
creation or an atomic task decision; `contained` is used only when an owned
probe/sandbox/evidence area is actually mutated; `preparedJournaledEffect`
requires the exact prepared/session/status digest before an authoritative
infobase effect; and `journaledEffect` requires the exact guard or recovery
digest. The latter two write intent before the external effect and verify or
reconcile its postcondition. None fabricates `dryRun`; an immutable sandbox
prepare/apply sequence exists only for a genuinely `contained` variant.
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
- a required manual-support target mode and a capability-proven exclusive
  configuration lease for its terminalization target; for a separate working
  infobase, also the exact human repository username plus history-visible
  computer/infobase identity and an internal service endpoint that performs the
  guarded inspection/lease;
- a unique profile-managed immutable recovery-distribution source for each
  support layer that may be observed, with exact layer identity/SHA, safe human
  display/leaf, manual-actor readability, and a WORM retention-provider
  capability held through task archive;
- validation, commit-comment, and cleanup policies.

The schema denies unknown fields and starts with this contract (environment
names are references, not secret values):

```yaml
schemaVersion: 1
branchedDevelopment:
  projectId: 9bd51fb1-c13f-475f-9fa7-f15578d67b3b
  workRoot: /absolute/task/work/root
  retentionProviders:
    local-worm-artifacts:
      kind: leaseBackedWormMount
      root: /absolute/user-visible/vendor
      leaseEndpoint: { env: UNICA_WORM_LEASE_ENDPOINT }
      token: { env: UNICA_WORM_LEASE_TOKEN }
      retentionCapabilityRowId: worm-mount-v1
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
      manualSupport:
        targetMode: reservedOriginal
        recoveryDistributions:
          - refId: vendor-main-8-3-27
            layerId: ОсновнаяПоставка
            vendorLayerIdentityDigest: 0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef
            expectedSha256: fedcba9876543210fedcba9876543210fedcba9876543210fedcba9876543210
            sourcePath: /absolute/user-visible/vendor/main.cf
            displayName: Основная поставка 8.3.27
            fileName: main.cf
            manualReadabilityCapabilityRowId: darwin-arm64-8.3.27.2074-en-file-file
            retentionProviderRef: local-worm-artifacts
            providerObjectRef: vendor-main-8-3-27-cf
      validationPolicy: full
      commitCommentTemplate: "{taskId}: {taskSummary}"
      cleanupPolicy: archiveThenQuarantineDelete
```

`workRoot` and explicit executable/repository/provider filesystem locations are
absolute. After canonical resolution, `workRoot`, durable state, coordination
root, original workspace, a File original-IB directory, and a file-repository
directory are pairwise non-overlapping: none may equal, contain, or be contained
by another. Retention-provider roots are pairwise non-overlapping with one
another and with all of those owned/protected roots. A recovery source must
resolve to the exact provider-owned object below only its designated provider
root, never through a symlink,
junction/reparse point, Git repository/worktree, filesystem/home root, or path
traversal. Neither the provider root nor a source may lie below a task instance,
quarantine, original IB/workspace, repository, durable-state, or coordination
root. Start applies these guards before it creates an instance, and archive
revalidates the live provider identities before retention-lease release. The
archive freezes those canonical provider-root/source boundaries. Cleanup always
compares every owned, quarantine, and destructive target with the frozen
boundaries in both containment directions; if a provider root/source still
exists after verified release, cleanup also canonicalizes and checks its live
identity. Its absence after that release is admissible because the external
owner may move/delete it; cleanup neither reconstructs nor touches the missing
object. Thus cleanup can never traverse into an original IB, 1C repository, or
profile-owned recovery CF even under a malicious nested profile.
`connectionSource: v8project` resolves the effective primary plus local overlay;
it never rewrites either file and imports only endpoint/options. Embedded
user/password values in the effective connection are rejected for this
workflow; credentials use the profile's environment references. Optional
infobase user/password references are omitted when the endpoint authenticates
without them. The repository account mode has only the safe `exclusive` value.
`manualSupport.targetMode` is required and has only `reservedOriginal` or
`separateWorkingInfobase`; there is no implicit default or per-operation
override. The reserved variant rejects all additional manual actor fields and
is unavailable unless its topology capability proves the exact-original
terminalization lease and verified release. The separate variant instead
requires `repositoryUser` and
`historyIdentity: { computer, infobase }`, all non-empty, and rejects equality
with the reserved username or original's capability-proven identity. These are
history-match identities, not credentials or a second connection string; Unica
never authenticates as the human actor. The separate variant additionally binds
an internal inspection/lease endpoint and secret references for the service
identity, not for impersonating that human. The endpoint may only inspect the
bound working infobase, acquire/release one exclusive configuration lease, and
report its durable receipt/postcondition. Missing capability to prove the exact
history fields, exclusive lease, or release makes the
profile/topology unavailable rather than inferred.
`branchedDevelopment.retentionProviders` is a closed, deny-unknown map. The
first implementation accepts only `kind: leaseBackedWormMount`; each provider
has one canonical absolute `root`, environment references for its owner-only
lease endpoint/token, and an exact `retentionCapabilityRowId`. That ID resolves
only in the tracked retention-provider manifest described below; it cannot name
a platform row or an arbitrary profile assertion. The row proves idempotent
task-scoped acquire/probe/release, stable object-to-file resolution, readability
while leased, and rejection of rename/overwrite/delete through archive. The
endpoint and token remain profile-only. A provider operation may change only
lease metadata; Unica never writes, replaces, moves, quarantines, or deletes
provider content.

`manualSupport.recoveryDistributions` is required and deny-unknown. Each entry
has a unique `refId` and exact `(layerId, vendorLayerIdentityDigest)` pair, a
64-hex expected SHA-256, an absolute local-only `sourcePath`, a bounded display
name, a safe `.cf` leaf, and a `retentionProviderRef` plus stable
`providerObjectRef`, plus a separate `manualReadabilityCapabilityRowId` that
resolves only to the exact platform row/case proving that Designer for the bound
actor can open this ordinary CF with the same SHA-256. It is never inferred from
or equated with the retention-provider row. The provider must prove that this
exact object resolves to the canonical source path and hash under its root; a
caller-selected path or a nearby provider object cannot be substituted. The
source path, provider endpoint, token, and provider object handle never enter
MCP results or persisted portable archives. Start/profile validation rejects duplicate/mismatched IDs,
unknown providers/kinds, unsafe or overlapping roots/paths, missing/unreadable
sources, non-distribution artifacts (including CFU), SHA drift, an actor-
unreadable source, or a provider/capability that cannot prove the lease and
overwrite/delete rejection through task archive. Before a manual authorization,
Unica acquires and receipts the task-bound retention lease, rechecks the same
object/path/SHA and actor readability, and repeats that proof immediately before
any corrective instruction. At runtime, every root-reachable layer must resolve
to exactly one current entry; a newly observed, missing, or stale layer makes
support preflight inconclusive rather than selecting a nearby artifact. Archive
releases each exact lease only after its handoff/evidence lineage is durable;
ambiguous release remains recovery-bound and cleanup cannot guess.
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
a durable changed/no-change receipt. A task no-change receipt selects one of the
closed equal legal phase pairs and preserves phase, evidence, cache, and replay
exactly; resolution receipts carry only the equal `synchronizationConflicts`
pair. A changed task-workspace receipt carries the
exact descendant invalidation closure; a changed resolution-workspace receipt
atomically supersedes only prior selectable receipts for the same conflict
target. If that target has a current decision, the same CAS transition moves
its immutable revision to `replacementPending` while leaving its consumed
receipt unchanged; the next current revision names it with
`replacesDecisionId`. Other conflict targets survive. A task-workspace mutation outside `developing` is
allowed only before any unresolved original effect/lock; it atomically returns
the task to `developing` and invalidates every descendant checkpoint, artifact,
merge decision/session, verification, integration/lock plan, and preview digest.
A resolution-workspace mutation instead produces a receipt bound to the
immutable base-session digest and workspace generation; it cannot change task
sources or phase. It and decisions share one evolving conflict-head CAS digest,
so the result has exactly one current decision per conflict while historical
decisions/consumed receipts stay auditable. Mutations are rejected throughout
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

The abandonment terminal path is `eligibleSafePhase -> archivedAbandoned ->
cleanedAbandoned`, or, when inverse support cleanup is required,
`eligibleSafePhase -> abandonmentReady -> archivedAbandoned ->
cleanedAbandoned`. The cleanup proposal preview and its distinct approved
archive apply both remain at the eligible phase; only the apply may publish the
manual authorization that then remains pending there.

`synchronizationPrepared` branches to `synchronizationConflicts` only when typed
conflicts exist; resolved decisions recreate the sandbox and return to
`synchronizationPrepared` before apply.

Support readiness is not a task phase. `merge.prepare(mode="mainIntegration")`
records one digest-bound `SupportPreflightOutcome`: `ready`,
`manualSupportRequired`, `vendorForbidsChanges`, or
`supportPreflightInconclusive`. The latter three are evidence-bearing stopped
results while the safe task phase remains `synchronized`; no main merge session
or lock plan is published. A `ready` observation is usable only while its
ordinary-result hash, canonical candidate set, support graph, capability row,
and repository/original anchors remain current.

`manualSupportCleanupRequired` is a separate staged archive stop, not a fifth
main-preflight outcome. `dryRun: true` preview persists only the inverse
transition proposal/evidence digest and preserves the exact eligible origin
phase; it publishes no authorization and performs no external lease acquire,
inspection, or release. A distinct apply requires `approvedPreviewDigest`,
journals before every external lease gate, and only after the root-reachable
recovery-handoff and mode-specific baseline gates pass publishes the
`awaitingArm` inverse authorization. Missing/stale evidence or a
capability-proven busy/dirty baseline returns the typed
`supportPreflightInconclusive` stop with no authorization/archive and the same
origin phase after verified release. An unknown acquire, inspection, or release
effect enters recovery instead of that retryable stop. Successful reconciliation enters
`abandonmentReady`, a terminal precursor that normally permits only status, a
typed routine repository refresh, and abandoned archive. A current cleanup
authorization narrowly also permits only its exact arming, reconciliation, or
cancellation; a frozen cleanup authorization permits only status and its exact
recovery. None of those exceptions reopens development or successful
integration. A routine advance keeps that phase only after the original is
repository-clean and task-only support state is recomputed; a remaining
transition produces a new cleanup proposal whose approved archive apply must
repeat the same journaled gates before publishing another authorization.

Blocking/recovery phases are `blockedByForeignLock`,
`staleRelevantBaseline`, `staleSupportPreflight`,
`lockPlanExpansionRequired`, `unexpectedDelta`, `validationFailed`,
`commitBlocked`, `recoveryRequired`, and
`committedUnverified`.
`synchronizationConflicts` is the single canonical conflict-state name.

An unfinished task can be deliberately abandoned through
`unica.branched.archive(outcome="abandoned", reason="superseded")`. It reaches
`archivedAbandoned` only after proving that no worker is active, the original
configuration equals repository content for every touched object, no task-owned
lock or pending/frozen support action remains, no task-only support transition
is left, and no external effect is unknown. Cleanup can then reach
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

If that edge discovers accepted task-only support changes, preview returns only
the inverse proposal/evidence digest and remains at the origin phase. The
distinct approved archive apply journals before external lease gates and alone
may create the inverse-only `awaitingArm` authorization instead of archiving.
Cancellation restores the bound origin/safe-ancestor phase; reconciliation or
its recovery uses the authorization's fixed `postReconcilePhase` and reaches
`abandonmentReady`, never an invented `localVerified` checkpoint.

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
| `synchronizationConflicts` | record one current decision head for every conflict; a same-target edit after a decision moves the historical head to `replacementPending`, and its replacement must carry the exact `replacesDecisionId`. Then `merge.prepare(mode="resolvedReplay")` replays only current heads, recreates the checkpoint sandbox, and returns to `synchronizationPrepared` only with zero remaining conflicts and no selectable receipt. Accidental/unbindable workspace changes instead require a digest-bound `supportedUpdate` replacement that atomically invalidates the old workspace/receipts/decisions only after a fresh session is durable |
| `staleRelevantBaseline` | original is still unchanged; `repository.unlock(reason="rollback")` must release the exact retained set and return `localVerified`, then a fresh Dn, synchronization, and plan are required |
| `staleSupportPreflight` | no task merge started and the original is either unchanged or has capability-proven clean repository-refresh evidence; `repository.unlock(reason="rollback")` releases the exact retained set, invalidates the main session/verification/plan/lock and stale gate, returns `synchronized`, and requires fresh main preparation/support preflight without reusing old history evidence. An unowned/unclassified original delta enters recovery instead |
| `lockPlanExpansionRequired` | original is still unchanged; exact full unlock returns `synchronized`, invalidates main preparation/plan evidence, and requires a new main sandbox and expanded plan |
| `unexpectedDelta` | record a digest-bound adapted-delta decision through `merge.resolve`, rerun `merge.verify`, or return to `developing`; never lock while unresolved |
| `validationFailed` | no original difference or owned lock may exist on entry; repair in the task workspace, whose next compatible mutation returns atomically to `developing` |
| `commitBlocked` | read-only external-state inspection followed by `repository.recover`; no retry |
| `recoveryRequired` | inspect the digest-bound exact recovery plan, then use `repository.recover` or documented external manual recovery followed by reconciliation; failed main validation may reach `validationFailed` only after original restoration and unlock are proven |
| `committedUnverified` | read-only reconciliation; transition only to `committedAndUnlocked` when both content and unlock are proven |
| `abandonmentReady` | normally status, typed routine repository refresh with support reclassification, or abandoned archive; a current cleanup authorization admits only its exact arming/reconciliation/cancellation, while a frozen one admits only status/recovery; neither reopens development/integration |

The stopped support outcomes have their own non-phase exits. From
`manualSupportRequired`, a human may create the exact root-only prerequisite
version described below; Unica then reconciles it, invalidates Dn-and-later
evidence, returns to `localVerified`, and requires a fresh distribution,
supported rebase, delta proof, and support preflight. From
`vendorForbidsChanges`, progress requires a changed task scope, a newer vendor
delivery that changes the restriction followed by the same refresh cycle, or
safe abandonment. `supportPreflightInconclusive` cannot be converted to either
of the other outcomes from localized prose or assumptions; the platform
capability/parser evidence must first become conclusive.

This naming and the success/abandonment terminal split deliberately supersede
the issue draft's inconsistent `syncPrepared`, `syncConflictResolution`,
`syncConflicts`, `archived`, and `cleaned` names. Safe abandonment supersedes
success-only cleanup only for disposable task data; `cleanedAbandoned` is never
repository success or completion of issue #137.

For every eligible phase above, guarded abandonment is an alternative to the
normal exit in the table. It is not available from `acquiringLocks`, `locked`,
`mainMerged`, `mainValidated`, `staleRelevantBaseline`,
`staleSupportPreflight`, `lockPlanExpansionRequired`, `committing`, `commitBlocked`,
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
| current `ready` support-preflight digest, `merge.verify(scope="mainSandbox")`, and approved `repository.planLocks` | `integrationPlanned` |
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
report/compare evidence. The preview records a complete contiguous history
partition, names every incoming modify/add/delete, derives the exact
`-Objects` selection plus existing root/target/parent/referrer lock closure, and
produces an immutable update digest. A history cursor is observation evidence,
not an invented Designer version selector: this design does not depend on a
non-existent version-pinned `-v` update.

Apply journals intent, acquires the configuration root first and then the exact
existing target closure in canonical order, and revalidates the partition,
support graph, clean original, and approved digest while those relevant objects
are frozen. It then invokes `/ConfigurationRepositoryUpdateCfg -Objects` for
only the approved selection, verifies the resulting exact object/reference
fingerprints, records the post-release cursor/partition, and releases every guard
in reverse order with proof. A relevant version that appeared before the guarded
recheck makes the preview stale; an unrelated concurrent version remains outside
the selection and is classified rather than silently ingested. A known foreign
guard lock is an evidence-bearing stop after compensation; an unknown
acquire/update/release effect enters recovery.

When, and only when, the approved selective plan contains added or deleted
repository objects, the adapter may additionally supply the platform's
`/ConfigurationRepositoryUpdateCfg -force` structural-change confirmation.
This is a derived adapter detail required to receive those exact objects, not a
public flag or a merge/conflict policy. Both selective-update locking and the
structural confirmation require their own capability fixtures and cannot be
reused for supported `/UpdateCfg`, merge, commit, unlock, reference clearing, or
administrative ownership override.

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
An intentional task transition into or out of `offSupport` is outside ordinary
branched development and the candidate/action schema rejects it. Any future
release workflow for such a transition requires its own ADR and public approval
contract; the separate frozen-recovery path may still inverse an accidental
action-attributed transition to reach its bound safe baseline. An exact target
that was already `offSupport` is not normalized or silently reattached: it may
be classified only as `preserveOffSupport`, which creates no support transition
and requires the sandbox/result support graph to preserve that pre-existing mode
exactly. A source already supported by its own distribution identity is
rejected; a different upstream vendor chain is preserved.

### Main-integration support preflight

Target editability is a mandatory gate inside
`unica.merge.prepare(mode="mainIntegration")`; it is not a second public tool.
The producer first builds and classifies the ordinary result CF, obtains the
platform `/CompareCfg` report against a repository-fresh original snapshot,
and joins it with the canonical UUID/property delta, development-object
ownership, add/delete semantics, and reviewed reference closure. XML changed
paths may accelerate that calculation or appear as audit evidence, but they
never define the candidate set. An unmapped path, unknown ownership kind, or
incomplete closure fails closed. CFU remains rejected and is never used to
probe support.

The complete candidate set is applied to a disposable clone of the original by
the real generated merge settings without `-force`. The platform documentation
states that `-force` can exclude objects restricted by support rules and
continue; therefore it cannot establish a complete successful merge. The
preflight combines the sandbox observation with a capability-proven support-
graph inspection and records, for each candidate and vendor layer, the current
support state, vendor delivery restriction, required transition, and evidence.
It publishes exactly one outcome with this precedence:

1. `supportPreflightInconclusive` when complete classification or diagnostic
   coverage is not proven;
2. `vendorForbidsChanges` when any exact candidate cannot become editable while
   retaining support, including a vendor-object deletion that would require
   `offSupport`;
3. `manualSupportRequired` when the non-empty exact human action is support-
   preserving: it may make every human-permitted blocker editable and/or restore
   task-created surplus editability no longer required after rebase;
4. `ready` only when every candidate is either already editable under support,
   positively proven `notApplicable`, or an exact pre-existing `offSupport`
   candidate classified `preserveOffSupport`; no task-created surplus
   editability or newly required detachment remains, and the no-force sandbox
   merge completes with the approved scope and exact support-graph preservation.

The gate digest binds the candidate manifest, ordinary-result hash, canonical
delta/reference closure, support graph, relevant-baseline/original fingerprints,
generated settings, sandbox result, and exact capability row. Before a manual
authorization is published, it also binds a verified ordinary vendor
distribution for every support layer reachable from the configuration-root
support-settings window plus the recovery-only capability
that can restore an accidental `offSupport` transition. For each layer Unica
also probes a profile-managed, retained, user-visible immutable CF source and
binds its logical reference, safe file name, actor, SHA, readability receipt,
and capability into the authorization. Unica never writes/deletes that external
source; no opaque artifact ID or raw path is left as the human recovery
mechanism. CFU cannot satisfy this
evidence; if any layer lacks it, the outcome is
`supportPreflightInconclusive`, so recovery cannot become an unreachable dead
end after the human action. The advancing
global repository-history cursor is carried separately. Its
`SupportGateHistoryEvidence` binds `gateObservedCursor`,
`classifiedThroughCursor`, and a complete partition whose `fromExclusive`
equals the former and `throughInclusive` equals the latter. Every entry must be
classified and its semantic-delta digest must match the corresponding typed
history observation. An old gate remains reusable only when the whole extension
is proven `unrelatedRoutine` and the recomputed relevant-baseline digest is
unchanged; any relevant tail invalidates Dn-and-later evidence and requires the
fresh-distribution path even if a later version restores the same bytes.
That reuse predicate also requires the exact root-reachable support-layer
identity set and recovery-distribution-set digest to remain unchanged. A
pre-terminal root/support advance that adds, removes, or changes a reachable
layer, or makes its retained distribution handoff non-current, cannot be hidden
inside an otherwise disjoint history tail: the old gate cannot authorize a
terminal effect, and a pending/frozen action must enter its typed
reconciliation/recovery or inconclusive path until complete current evidence is
bound. No further human recovery instruction may use the stale handoff set.

Main-session publication, main-sandbox verification, lock planning, lock
acquisition, and original merge all bind and CAS the current history-evidence
digest as well as the semantic gate digest. A non-anchor gate-input mismatch
stops as `supportPreflightStale`; before locks it returns to `synchronized`,
after the root guard or full set is acquired it enters
`staleSupportPreflight` until exact compensation/unlock returns to
`synchronized`. A relevant history partition instead has
`relevantBaselineChanged` precedence and returns to `localVerified` for fresh
Dn/rebase. `originalFingerprintChanged` is a gate-only mismatch only with a
capability-proven clean repository refresh and proof that no task merge began;
an unowned/local/unclassified original delta enters recovery. A stale gate never
degrades to a warning or generic task-repair `validationFailed` result.

For `manualSupportRequired`, the task's target and repository-account
coordination reservations remain current, so another Unica task cannot claim
the reserved original/account. Unica nevertheless has no automated worker,
service-held configuration lease, or Unica-owned repository lock/effect while
the user acts. The skill shows the exact objects/layers and persists a
digest-bound manual-action authorization that is not merge readiness, carries
that immutable per-layer recovery-distribution set, and can be consumed only by
prerequisite reconciliation.
The validated project profile selects, rather than the caller choosing ad hoc,
one of two manual targets:

- `reservedOriginal`: a bounded journaled human window uses the exact reserved
  repository account and original infobase. The account has a capability-proven
  empty lock baseline, and Unica has no automated worker or owned lock while the
  human acts;
- `separateWorkingInfobase`: the authorization binds an exact manual repository
  actor and repository-bound working infobase distinct from the reserved account
  and original. Before exposing the instruction, the service acquires its
  exclusive lease, proves it clean and repository-equal, persists the complete
  authorization baseline, and proves lease release; busy, dirty, or unknown
  inspection publishes no authorization. The reserved original fingerprint cannot move until typed
  reconciliation updates it. Every reconciliation, cancellation, or frozen
  recovery terminalization also requires the profile-bound service to acquire
  an exclusive configuration lease on that exact working infobase.

`reservedOriginal` has the corresponding local-state race closure at
terminalization rather than while the human edits. The user must first close
Designer; then the profile/capability service acquires an exclusive
configuration lease on the exact reserved original and holds it from guarded
inspection through authorization consumption, cancellation, or frozen-recovery
finalization. Success requires a durable capability receipt and verified lease
release. A busy, dirty, missing, or ambiguous lease proof cannot terminalize the
authorization. This configuration lease is distinct from the persistent task
coordination reservations and from the short repository root guard.

The manual action is armed in two stages. In either mode the skill instructs the
human to:

1. lock only the configuration root, without editing support settings or
   committing;
2. resume through
   `repository.update(mode="supportPrerequisiteArm")`. Its preview is strictly
   read-only, accepts no `operationId`/`dryRun`, creates no durable preview
   handle, and is repeated after response loss. The separate `localJournaled`
   apply supplies `approvedArmingDigest`, proves that the complete history from
   the authorization cursor is still gap-free and unrelated, the exact bound
   actor currently owns the root, and the support graph, original fingerprint,
   and retained recovery handoffs are unchanged, then durably publishes an
   arming receipt and the edit instruction;
3. follow that instruction by retaining the root and applying only the armed
   transitions: make listed candidates editable with support retained, or
   restore rules/capability previously opened by this task but no longer
   required;
4. commit and release the root as a separate prerequisite repository version,
   then close the profile-bound Designer session;
5. resume with `branched.status` and typed prerequisite reconciliation.

Any root/support, original, or handoff drift before arming permits no edit and
requires a fresh preflight. Staleness found by either the read-only preview or
the approved apply's final recheck leaves the authorization `awaitingArm`;
neither path arms or cancels it and neither can publish an arming receipt. After
the human releases any held root, only a separate
`repository.update(mode="supportPrerequisiteCancellation")` may cancel it, and
that flow must repeat the complete authorization-anchored proof and durably
publish its own cancellation receipt. The authorized version must be the first
root/support version after the arming cursor, use the exact bound actor/IB, have
the exact armed delta, and have no intervening root/support version. The edit
instruction asks the actor to retain the root, but continuous ownership is not
a machine-verifiable acceptance fact: release/reacquire without an intervening
root/support version remains semantically admissible. Thus a newly introduced
vendor layer can never be edited under recovery evidence frozen for an older
support graph.

A root/support version committed while the action is still `awaitingArm` is
never retroactively authorized, even when actor, IB, and delta exactly match
the proposal. With complete evidence, explicit cancellation classifies it as
`preArmExternal`, preserves it as the new external baseline, cancels with no
arming fields, and returns to the relevant-advance/fresh-preflight path. With
incomplete evidence cancellation remains inconclusive. This pre-arm path never
enters the receipt-required armed recovery union and therefore cannot dead-end
between stale arming and an impossible normal reconciliation. If the explicit
cancellation itself has an unknown root-guard, mode-lease, original-update,
authorization, or release effect, the action freezes only as
`preArmCancellationEffect`: no arming receipt or support-recovery disposition is
created. Its first recovery is observation-only; a conclusive observation
produces a new digest and requires separate approval before exact cancellation
finalization. That immutable plan assigns one intent/receipt to each missing
effect and permits only root acquisition, mode-lease acquisition, a stage-
sensitive under-guard recheck, optional selective original update,
authorization cancellation, mode/root release, and a distinct terminal local
recovery receipt in that order. Effects already proven by the interrupted
operation are not repeated. Pre-update drift can create a fresh plan only after
receipt-proven reverse compensation and preservation of the old attempt audit;
drift inside an update-ready continuously guarded boundary is a capability
breach, not a no-effect replan. A known foreign root or busy/dirty mode lease is
a typed compensated stop whose complete evidence and exact external instruction
remain digest-covered in the fresh current recovery plan for status
reconstruction after response loss, while an unknown acquire/release remains recovery.
The workflow is enabled only by a fixture proving both guards survive worker/
connector death until explicit receipt-bound release. Terminal status/archive
retain the distinct cancellation/recovery receipt pairs, full immutable
finalization plan with prior compensated-attempt audits, completed progress with
full receipts, and the full effect/recheck/selective-update/post-release
lineage. This is recovery of the cancellation effect, never
retroactive permission to edit.

No candidate object is needed for this support-only version. In
`reservedOriginal` the authorization binds the complete empty reserved-actor
lock baseline and reconciliation requires the post-action set to remain empty;
a free root alone is not proof that this same-user window is closed. In the
separate mode any distinct-human non-root lock is handled later as an ordinary
foreign-lock stop, but the root must still be released. `reservedOriginal` is
an explicit journaled exception to the dedicated-account exclusivity rule, not untracked
out-of-band use. On resume the skill calls `branched.status` first.
The external version necessarily stales the old support-gate anchor; its
separate pending action authorization survives only for classifying exactly one
allowed root delta plus explicitly partitioned routine versions and cannot
create a session, plan, or lock.
The support-prerequisite variant of `repository.update` then uses repository
history plus semantic comparison to prove the human version changed only the
configuration-root support graph by the previously allowed transitions, names
its repository actor/version/working infobase, proves root release and, for the
reserved mode, the complete actor lock set returned to its empty baseline, and
updates the original to a clean repository state when needed. A complete
read-only snapshot may improve preview diagnostics, but every apply journals and
acquires the root, rechecks history/support/original under that guard, consumes
or cancels the authorization atomically, and releases/verifies the guard.
Prerequisite reconciliation selectively refreshes only the locked configuration
root; classified concurrent non-root routine versions remain unapplied input for
the next ordinary refresh and force the bound relevant-advance phase when
applicable. Cancellation selects the root only when a classified external-
support state must be preserved, otherwise its selective set is empty. The
history cursor is never passed as an invented `-v` selector.

Every prerequisite/cancellation history partition starts exactly after the
authorization's expected-before cursor, is gap-free through the under-guard
terminalization anchor, and frozen recovery retains the same lower bound. The
consumed/cancelled/finalized authorization state and its immutable receipt are
durable before guards are released. Versions first observed after that durable
terminalization belong to a post-release result-phase partition: they may select
the already-bound safe advance phase or create a new typed gate/recovery, but
they cannot reopen, mutate, or suppress the terminal receipt of the old
authorization. In particular, a later authorized/invalid/corrective/unattributed
support entry is classified in a new follow-up context rather than being used to
pretend the old authorization remained pending. After response loss, status
therefore exposes both the terminal receipt and any distinct current follow-up
handle.

In `separateWorkingInfobase`, apply acquires the exclusive working-infobase lease
while the repository root guard is held, proves that its current configuration
equals the exact terminal repository state derived from the authorization
baseline plus the accepted/corrective/preserved sequence, with no local support
or uncommitted configuration delta, and holds the lease through durable
terminalization. It then releases and verifies both guards. A known-busy lease or
an acquired-but-dirty working infobase returns the tagged
`manualSupportLocalChangesRemain` stop, performs no terminalization, proves root
compensation, and leaves the authorization unchanged. The busy variant has no
lease receipt and returns exact owner/busy evidence; the dirty variant returns
the acquired lease/fingerprint/release proof. An unknown lease effect enters
armed support recovery, or the distinct no-arming pre-cancellation recovery when
it interrupted cancellation from `awaitingArm`. Unica never resets or discards
a human working infobase.

Reconciliation never accepts a generic “done” assertion. Unexpected root properties, object content,
support-layer identity, `offSupport`, a retained lock, an un-attributable
version, or a cross-mode actor/original/working-infobase observation stops
reconciliation.

If the user abandons the manual action before committing it, neither a task
mutation nor archive silently cancels the pending authorization. The explicit
preview/apply `repository.update(mode="supportPrerequisiteCancellation")`
variant must prove no attributable version, unchanged original/support graph,
and a closed lock window before atomically cancelling it. Fully classified
intervening routine versions are retained in the partition but are not applied
merely to close the authorization; relevant ones select the bound
relevant-advance phase, while unrelated ones may preserve the cancelled phase.
A proven disjoint external-support root is selectively applied and preserved.
An observed but wrong immutable
version freezes the authorization and enters reconcile-only recovery; later
corrective versions remain in the archived history rather than rewriting the
evidence.

Frozen support recovery has exactly three disposition-bound results:

1. `restoreThenReauthorize` inverses only invalid deltas proven attributable to
   this action, cancels the authorization, and returns to its bound cancelled
   phase for fresh authorization;
2. `preserveExternalAndReauthorize` never reverses a capability-proven
   wrong-actor/wrong-target version. It preserves that external baseline,
   cancels this authorization, and returns only to the relevant-advance phase
   for fresh Dn/rebase/preflight. Overlapping or initially unclassifiable
   external support remains a typed coordination conflict until an external
   corrective version or immutable ownership receipt resolves it;
3. `restoreThenAbandon` is mandatory for action-attributed unauthorized content
   or `offSupport`. It restores the disposition-bound safe content/support
   baseline, permanently forbids successful integration, and reaches only
   `abandonmentReady`.

All three dispositions preserve every completely classified routine version,
every proven disjoint external-support version, and every additional
disposition-preserved externally owned version in history order. No version is
both treated as external and automatically inverted; provenance is resolved
before mixed mismatch kinds select a disposition. The recovery digest binds the
complete cursor range, per-version semantic observations, resulting baseline,
phase, external instruction, and any forbidden-success literal.

Terminal support recovery uses a persisted finalization plan independent of
whether a corrective instruction was needed. It acquires the configuration root
first and then only the exact content/parent/referrer targets, rechecks complete
history, support graph, content, original, and corrective before-state under
those locks, performs the exact selective original refresh, and atomically
cancels or abandonment-finalizes the frozen authorization before reverse release
proof. `reservedOriginal` additionally restores the reserved actor's empty lock
inventory; `separateWorkingInfobase` additionally uses the same exclusive lease
closure above. A known foreign finalization lock stops as
`supportRecoveryBlockedByLock` after compensation; incomplete correction or
external conflict remains a typed pending recovery; any unknown effect stays
`recoveryRequired`.

The same immutable terminal-anchor/post-release rule applies to each recovery
disposition: later history may determine a new safe phase or follow-up recovery,
but never retroactively changes a cancelled or abandonment-finalized
authorization back to its pre-terminal state.

A wrong or incomplete corrective version remains `supportCorrectionPending`
with an exact external instruction. Any newly appended immutable history, any
accepted ownership evidence that reclassifies a previously
`invalid`/`unattributed` observation, or separate-working-IB closure
materialization recomputes the finalization/recovery digest. The first
observation returns `supportRecoveryReapprovalRequired` with no repository
effect; only a fresh explicit approval of that exact digest may start
finalization locking. Ownership evidence cannot reclassify action-owned,
external-owned, authorized, routine, corrective, or otherwise positive history;
action-owned content/`offSupport` taint permanently keeps its abandonment
precedence. Before a corrective version exists, the separate-working-IB closure
plan contains only the desired fingerprint/support destination, never a
fabricated future repository cursor or object-version map; complete observed
history materializes those fields and triggers the same fresh-digest reapproval
boundary.

An accepted main-integration prerequisite is a relevant repository advance. It is archived
separately from the task-content commit and atomically invalidates the current
refresh distribution, supported-update/main sessions, decisions,
verifications, plans, and previews. The task returns to `localVerified`; Unica
must create and verify a fresh Dn, repeat the supported three-way update and
delta proof, and run a new support preflight. Merely rerunning the old sandbox
or lock plan is forbidden. Other working infobases receive the prerequisite by
their normal repository update process.

After every rebase, the new gate compares current exact candidate/layer needs
with all support transitions previously authorized by this task. Any object or
configuration capability left more editable than the new task delta requires
is a surplus transition. The outcome is again `manualSupportRequired`, now with
the exact inverse restore transitions; `ready` is impossible until another
separately authorized root-only version restores them and the refresh cycle is
repeated. Thus a task can have several audited prerequisite/corrective versions,
but each authorization permits exactly one attributable human root-only
version and no unrelated content.

A pending authorization is an unresolved external-coordination barrier. Task
mutation or abandonment remains rejected until the explicit cancellation flow
proves no matching external version/change/lock and cancels it; otherwise
reconciliation is mandatory. If the task is abandoned after one or more accepted support changes,
`branched.archive` requires authorized inverse root-only cleanup and
reconciliation before `archivedAbandoned`; ordinary branched development never
silently retains task-only editability.

Every final lock plan includes the configuration root as an explained
`supportGraphGuard` in addition to the exact development objects. This freezes
both presence and absence of support and prevents the accepted support graph from changing
between `ready` and original merge. The capability fixture must prove how an
unchanged guard lock is released together with the task commit without adding
unrelated repository content; otherwise the topology is not enabled. The root
guard is acquired first and the support gate is rechecked while it is held;
only then may object locks be acquired. A mismatch releases/verifies the root
guard before returning a stale-gate result.

Successful original merge is the one expected original-fingerprint change: it
atomically consumes, rather than invalidates, the `ready` gate and binds that
historical gate to the merge receipt and authorized post-merge fingerprint.
Post-merge verification and commit use only that lineage. Any other pre-effect
or post-effect fingerprint change stales the gate or requires recovery. Every
non-anchor stale result names the exact changed gate inputs and returns full
expected/observed digests plus the endpoint-bound history evidence; a relevant
history tail uses the distinct fresh-Dn path. Commit preview and its immediate
pre-effect guard remeasure the consumed gate/receipt/post-fingerprint lineage
and post-merge partition; drift starts no commit and enters the exact
restore-plus-unlock recovery.

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
second prepared manual receipt. A changed receipt supersedes only prior
selectable same-target receipts and atomically changes the digest when it moves
a current decision to `replacementPending`; it never rewrites the consumed
receipt behind that historical decision. Each replacement decision
compare-and-swaps the decision-set digest and carries `replacesDecisionId`; the
server journal retains all revisions but owns a single canonical current-head
order. Resolved replay binds both digests, rejects undecided/replacement-pending
conflicts or unbound workspace changes, replays the complete current-head
projection without a caller-supplied order, then destroys the old resolution
workspace.

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
after a `ready` support-preflight result, the sandbox result, support isolation,
lock plan, and relevant anchors are verified.

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
evidence, and returns to `localVerified`. Target editability restrictions use
the four support-preflight outcomes and never masquerade as task repair. An
unexpected scope, conflict, support-isolation violation, or extra repair stops
as `mainPreparationMismatch` in `validationFailed` with immutable
comparison/difference evidence and no main session. Neither path enters a
conflict state with no legal exit.

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
contract and forces manual recovery when observed. The sole exception is a
current `reservedOriginal` support-action authorization: it journals an exact
human window and permits only the bound root lock, support transitions, one
root-only commit, and release. A frozen recovery authorization is the only other
exception: it permits the reserved account to acquire the root plus the exact
digest-bound restoration/finalization targets, never a broader subtree, and must
restore the empty actor-lock inventory before terminalization.
`separateWorkingInfobase` instead requires its
bound distinct actor/infobase. Both modes are accepted only through typed,
read-only-first reconciliation; the reserved mode additionally requires the
complete actor-lock inventory described above. Unica never holds a root lock while waiting indefinitely for interactive
support editing.

This mandatory exclusive-account rule for automated lock operations, the
explicitly bounded manual exception, and the absence of a forced-unlock API
supersede the issue draft's unjournaled conditional same-user proposal and
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

The final task-content commit uses the exact verified integration set once,
including additions and deletions that have no independently acquired lock,
with the frozen policy-rendered task comment, without `-keepLocked` and without
`-force`. The earlier human root-only support prerequisite, when required, is a
separate audited repository version and contains no task business change.
Preview and the immediate pre-effect guard scan every version after the original
merge receipt, recompute the integration/reference/support closure, and bind
`PostMergeHistoryGuardEvidence`. Its partition starts exactly after the merge
receipt cursor, ends at its classified-through cursor, and may contain only
versions proven unrelated to that closure.

Commit is enabled only when a real capability fixture proves the platform's
atomic safety boundary: with the exact root/target locks held, the no-force
commit either rejects a concurrent change to a locked target/root or a new
reference that blocks an approved deletion before any task-content commit, or
commits the exact set once. Other capability-proven harmless closure expansion
may commit concurrently and is retained as `nonConflictingConcurrent`; it is
not mislabeled as unrelated. This is a proven lock/commit-validation property,
not a claimed global history-CAS or version-pinned `-v` switch. A pre-intent
referrer or a concurrent referrer that changes a locked target/root or blocks an
approved deletion starts no task-content commit and enters the exact
original-restore/full-unlock recovery. Success requires repository content
equality and proof that every acquired task/support-guard lock is released.
Partial or ambiguous results are not retried and cannot be archived as
successful.

### Capability gating

Unica ships two tracked, non-interchangeable capability manifests. Platform and
Designer behavior is recorded in
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
      "commit-atomicity", "commit-concurrent-lineage-safety",
      "supported-update", "support-isolation", "support-preflight",
      "manual-support-prerequisite", "manual-working-infobase-lease",
      "manual-recovery-handoff-readability",
      "support-recovery-finalization", "rollback", "multi-vendor-support",
      "manual-replay", "repository-selective-update",
      "repository-structural-update"],
    "evidence": {"path": "darwin-arm64-8.3.27.2074-en-file-file.json",
      "sha256": "2222222222222222222222222222222222222222222222222222222222222222"},
    "passed": true
  }]
}
```

Every recovery-distribution profile entry's
`manualReadabilityCapabilityRowId` resolves only to one exact row in this
platform manifest, and that row must contain the exact
`manual-recovery-handoff-readability` case. The retained evidence proves that
Designer for the bound manual actor can open the same ordinary CF identified by
the handoff SHA-256. Provider-side visibility of bytes is insufficient and
cannot satisfy this platform capability.

Retention-provider behavior is recorded separately in the tracked
`plugins/unica/references/branched-development/retention-provider-capabilities.json`.
Every profile `retentionCapabilityRowId` resolves only in that file. Each row is
closed and has exactly:

```text
{ id, schemaVersion, featureContractVersion, contractDigest,
  host { os, arch }, providerKind, providerVersion,
  storageKind, storageVersion, harnessDigest, implementationCommit, passedAt,
  cases: [{ caseId, resultDigest, postconditionDigest }],
  evidence { path, sha256 }, passed: true }
```

The case evidence records and the `{ path, sha256 }` evidence locator are
closed, case IDs are unique and canonical, and the exact required case set
proves:

1. idempotent task-scoped lease acquire and response-loss replay;
2. exact observation that the same lease is held for the bound provider object;
3. manual-actor visibility/readability of the bound SHA while held, without
   claiming the separate Designer-open capability;
4. denial of rename, overwrite, and delete while held;
5. exact-once release and response-loss replay;
6. reconciliation of unknown acquire/observe/release effects without guessing;
7. canonical root/object containment with traversal, symlink, junction, and
   reparse-point rejection.

The row's evidence is tracked, redacted, and machine-readable. CI recomputes its
evidence SHA-256, contract digest, harness digest, exact case set, and pass
status. Before task creation `branched.start` rejects a missing or stale row, a
profile/live host/provider/storage kind or version mismatch, a skipped or
duplicate case, an evidence/digest mismatch, or any unpassed evidence. Neither
manifest may resolve the other manifest's capability-row ID.

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
    structural-change confirmation;
11. complete support-preflight classification for ready, human-editable,
    vendor-forbidden, and inconclusive cases without `-force`;
12. both profile-bound manual target modes, root-only prerequisite attribution/
    cancellation/reconciliation, reserved-actor lock inventory, optional
    read-only root observation plus mandatory apply guard, separate-working-IB
    exclusive-lease busy/dirty/unknown outcomes, all three invalid-history
    recovery dispositions and finalization guards, inverse abandonment cleanup,
    full Dn refresh, first-acquired support-graph guard/recheck, and release of an
    unchanged guard without unrelated repository content;
13. contiguous history-partition endpoint/digest binding, root-first exact
    target locking plus selective `-Objects` repository refresh without a
    version-pinned selector, and atomic commit safety against a concurrently
    added deletion-blocking referrer or locked-target/root change while
    permitting unrelated commits and classifying harmless closure expansion as
    `nonConflictingConcurrent`.

Every row contains all timeout keys shown above. Missing/zero/out-of-range keys
invalidate the row. Evidence includes forced deadline termination and
postcondition/recovery results for repository update, one lock call, the whole
lock transaction, commit, rollback, and one sandbox operation; documentation or
a process exit alone does not prove safe timeout handling.

Items 7-13 are deliberate corrections added by design review: the original six
spikes did not close rollback, multi-vendor support, manual materialization,
repository structural-update confirmation, or safe target-support preparation.
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
variants, add the skill scenario, all 21 lifecycle tools, ordered transcript
guards, and generated-package smoke while preserving the single server named
`unica`. A package that advertises a compatible BSL writer additionally proves
its branched-task schema, receipt, no-op/replay, and manual-conflict binding; a
package without one proves that only BSL-writing scenarios stop and that the
skill never substitutes shell/direct-file mutation.

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
- [1C branched configuration development and root-only support-rule locking](https://kb.1ci.com/1C_Enterprise_Platform/Guides/Developer_Guides/1C_Enterprise_Development_Standards/Creating_and_modifying_metadata_objects/Configuration_operation_arrangement/Branched_configuration_development/)
- [1C configuration delivery and support in collaborative development](https://kb.1ci.com/1C_Enterprise_Platform/FAQ/Development/Standards/Configuration_delivery_and_support/)
- [1C distribution commands](https://kb.1ci.com/1C_Enterprise_Platform/Guides/Administrator_Guides/1C_Enterprise_8.3.27_Administrator_Guide/Appendix_7._Startup_command-line_options_of_1C_Enterprise/7.4._Running_Designer_in_batch_mode/7.4.7._Commands_for_creating_distribution_and_update_files/?language=en)
- [1C supported configuration update](https://kb.1ci.com/1C_Enterprise_Platform/Guides/Administrator_Guides/1C_Enterprise_8.3.27_Administrator_Guide/Appendix_7._Startup_command-line_options_of_1C_Enterprise/7.4._Running_Designer_in_batch_mode/7.4.6._Configuration_support/?language=en)
- [1C compare and merge commands](https://kb.1ci.com/1C_Enterprise_Platform/Guides/Administrator_Guides/1C_Enterprise_8.3.27_Administrator_Guide/Appendix_7._Startup_command-line_options_of_1C_Enterprise/7.4._Running_Designer_in_batch_mode/7.4.4._Configurations_and_extensions/?language=en)
- [1C configuration repository commands](https://kb.1ci.com/1C_Enterprise_Platform/Guides/Administrator_Guides/1C_Enterprise_8.3.27_Administrator_Guide/Appendix_7._Startup_command-line_options_of_1C_Enterprise/7.4._Running_Designer_in_batch_mode/7.4.15._Configuration_repository_operations/?language=en)
