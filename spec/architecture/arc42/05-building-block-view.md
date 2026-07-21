# 5. Представление строительных блоков

## Top Level

- `interfaces::mcp`: stdio MCP transport, JSON-RPC methods, tool list and call
  response mapping.
- `application`: `UnicaApplication`, `ToolSpec`, `ToolHandler`,
  `OperationResult`.
- `domain`: `WorkspaceContext`, `DomainEvent`, `CacheImpact`, `CacheReport`.
- `infrastructure`: command adapters, standards adapter, package launchers, and
  `WorkspaceStateRepository`.
- `branched-development`: task state, canonical delta, merge decisions, lock
  planning, repository integration, archive, and recovery.

## Domain Blocks

- `WorkspaceContext` discovers cwd, workspace root, cache root, and workspace
  epoch.
- `DomainEventKind` describes state-changing facts such as `FormChanged`,
  `BuildCompleted`, and `SourceSetChanged`.
- `CacheImpact` maps events to invalidated and eagerly refreshed cache names.
- Branched domain models define phase transitions, canonical UUID/property
  deltas, ownership/reference closure, four-outcome main support preflight,
  attributable profile-bound root-only support prerequisites with
  `awaitingArm`/`armed` state and immutable arming receipts, plus cancellation/
  recovery/cleanup paths, global history cursors, relevant-baseline digests,
  complete semantic partitions, `staleSupportPreflight`, exact recovery lock
  targets/three dispositions, support layers, compensated locks, and terminal
  proofs without filesystem/process access.
- The same models define RFC 8785 JSON contract digests and domain-separated
  operation inputs, the exact four-state operation record, closed missing-task
  reservation blockers, and `RepositoryTargetIdentity` for both root and
  development-object lock/conflict targets.

## Application Blocks

- `tools()` is the canonical public tool registry.
- `call_tool()` resolves dry-run semantics, workspace context, adapter dispatch,
  event emission, and cache reporting.
- Mutating tools with an ordinary honest preview default to `dryRun: true`;
  mutations without one follow their closed-variant policy rather than one
  generic prepare/apply boundary. `localJournaled` records owned local creation
  or an atomic task decision with no external effect; `contained` alone owns
  actual probe/sandbox/evidence mutation; `preparedJournaledEffect` binds an
  exact prepared/session/status digest; and `journaledEffect` binds an exact
  guard or recovery digest.
  The two external-effect policies write intent before effect and verify or
  reconcile their postconditions; none fabricates `dryRun`.
  The `supportPrerequisiteArm` preview is a closed `readOnly` exception with no
  `operationId`, `dryRun`, or durable preview handle; its apply is
  `localJournaled` and requires `approvedArmingDigest`.
- Branched use cases expose an exhaustive execution policy per closed request
  variant (with a tool default only when uniform), require stable operation IDs
  for mutations, and extend results with typed workflow evidence/data. They
  preserve unrelated/external history, enforce conditional
  `abandonmentReady` actions, and never infer relevance from cursor movement.
- The `repository.update` use case owns the separate target-effect-free
  `supportPrerequisiteArm` read-only observation and `localJournaled` apply.
  The preview is repeated after response loss; apply proves the bound actor's
  current root ownership and unchanged authorization inputs before publishing
  any edit instruction. Stale at either stage leaves `awaitingArm` unchanged;
  after root release, only the fully proven cancellation variant may cancel.
  Interrupted awaiting-action cancellation recovery owns the closed one-effect/
  one-receipt finalization sequence and stage-sensitive under-guard recheck;
  pre-update replans require reverse compensation/audit, while protected drift
  is a capability breach. It is enabled only with crash-stable root/mode guard
  evidence and publishes distinct cancellation and terminal recovery receipts.
  Reconciliation then requires the exact receipt, actor/IB/delta, no intervening
  root/support version, and the first such version after the arming cursor; it
  does not require evidence of continuous root ownership.

## Infrastructure Blocks

- Native operation handlers implement XML/DSL backend behavior inside
  `unica-coder`.
- `CliAdapter` invokes checksum-wrapped bundled tools.
- `StandardsAdapter` is the internal standards boundary and must become the real
  HTTP MCP client before closing the standards gap.
- `WorkspaceStateRepository` persists volatile cache state under the configured
  cache root.
- `BranchedTaskRepository` persists schema-versioned operational records under
  the durable state root; it is not an extension of `WorkspaceStateRepository`.
- `BranchedTargetLocator` persists the project/target-to-state-root registration
  and start-attempt replay records under a non-overridable owner-only
  coordination root.
- `BranchedReservationCoordinator` owns atomic target-plus-account reservations.
  The owner-only coordination root/mutex remains its host-local fast guard; a
  capability-proven shared linearizable backend is mandatory whenever either
  endpoint is multi-host.
- `DesignerPort` is a typed platform-neutral boundary. Its OS-specific locator,
  process, encoding, service-message, and filesystem implementation lives under
  `infrastructure/platform/**`. Repository refresh locks root plus the exact
  existing target/parent/referrer closure and uses an approved selective
  `-Objects` set plus per-target revision/fingerprint proof; the ignored/latest
  `-v` behavior is not exposed as version-pinning. Final commit is enabled only
  for a capability row that proves the immediate no-force atomic-safety guard.
- Typed manual-infobase inspection ports resolve only profile secret references,
  acquire a capability-proven exclusive configuration lease on the exact
  original or separate working IB selected by the support authorization, and
  report busy/open/dirty state. They cannot reset or discard the human IB, and
  an unknown post-authorization lease effect remains bound to that same support-
  prerequisite recovery.
- `RetentionProviderPort` resolves one deny-unknown profile provider/object to
  the exact canonical recovery CF and SHA, and owns only idempotent task-scoped
  lease acquire/probe/release metadata. Its capability proves actor readability
  and rename/overwrite/delete denial through archive; it cannot write, replace,
  move, quarantine, or delete provider content.
- `SecretResolver` resolves references only in memory. `TaskWorkPolicy` owns
  instance markers, containment, quarantine, and destructive cleanup guards,
  including pairwise provider/source/owned/protected-root exclusion.
- A detached branched-operation worker owns long `operationId`-bound contained
  or authoritative Designer calls across MCP disconnect and records effect
  barriers. Read-only inspections instead use bounded ephemeral processes and
  never create durable operation, lease, start-attempt, receipt, preview/evidence
  handle, or task/status state.

## Target Native MCP Handlers

The target implementation for configuration, form, DCS, MXL, role, subsystem,
interface, and template operations is native Rust logic behind `unica.*` tools.
Python/PowerShell/Bash operation files must not remain as runtime building
blocks. Reference scripts belong in test fixtures only.

The pinned `v8-runner` remains an internal adapter for its existing runtime
contract; it is not the implementation of supported update or repository tools.
