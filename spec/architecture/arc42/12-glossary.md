# 12. Глоссарий

- Adapter: internal boundary that calls a bundled tool or remote endpoint.
- Cache impact: the set of cache names invalidated or refreshed by domain events.
- Capability row: retained proof that one exact host OS, 1C platform
  family/version, and locale passed every required real repository scenario.
- Canonical delta: semantic configuration change keyed by metadata UUID and
  property path, independent of raw XML file counts.
- Coordination locator: owner-only non-overridable record that binds canonical
  project/target identities to one durable state root and unresolved history.
- Distribution: full CF created by `/CreateDistributionFiles -cffile` and
  proven to establish vendor support when deployed.
- CFU: configuration-update artifact that Unica may classify only to return a
  typed rejection; it is never a baseline, comparison, merge, or deployment
  input in the branched workflow.
- Instance ID: internal UUID used for one disposable task cycle and its paths;
  distinct from the external tracker task ID.
- Integration set: verified add/modify/delete repository object set bound to
  merge, validation, acquired-lock, and commit digests; it includes new objects
  that cannot themselves be locked before creation.
- Lock plan: explained closure of repository development objects needed for the
  canonical delta, ownership, additions/deletions, and references.
- Operation ID: caller-stable idempotency key bound to one canonical mutating
  request and durable result.
- Project ID: stable UUID in local Unica configuration used to locate durable
  task state across project relocation; it is not the mutable workspace epoch.
- Repository transport: file or server access topology, included in a capability
  row independently of original-infobase kind.
- Repository history cursor: immutable global history scan position. Its advance
  does not by itself say that task-relevant content changed.
- Relevant baseline digest: digest of the task candidate/reference closure,
  repository content/ownership, and support root/layers on which it depends.
- Original clean refresh proof: capability-bound evidence that an unexpected
  original fingerprint is nevertheless a clean repository refresh and no task
  merge started; without it the delta is recovery, not retryable gate staleness.
- Repository history partition: complete contiguous classification of every
  version between two cursors as unrelated/relevant routine, authorized or
  external support, invalid, or corrective history. For a manual support action,
  arming, reconciliation, cancellation, and frozen recovery all begin at that
  authorization's expected-before cursor.
- Selective repository update: capability-proven original refresh restricted to
  the approved exact `-Objects` set while root plus the existing target/parent/
  referrer closure is locked, and verified by a per-target revision/fingerprint
  map; `/ConfigurationRepositoryUpdateCfg -v` is not a pin.
- Support preflight: digest-bound main-integration observation over the complete
  canonical candidate set, target support graph, no-force sandbox merge,
  anchors, and capability row; its outcome is ready, manual, vendor-forbidden,
  or inconclusive.
- Support prerequisite version: separately audited human repository version
  that changes only the configuration-root support graph by transitions
  approved by a prior manual support-preflight or inverse-abandonment cleanup
  result. It is accepted only from an armed action as the first root/support
  version after the arming cursor; several immutable corrective versions may
  form one archived chain.
- Support action arming receipt: immutable evidence produced only by the apply
  half of `repository.update(mode="supportPrerequisiteArm")` after proving the
  authorization-anchored all-unrelated prefix, unchanged candidate/relevant
  baseline/support/original/recovery handoffs, and the bound actor's existing
  root lock. The observation half is strictly read-only, has no `operationId`,
  `dryRun`, or durable preview handle, and is repeated after response loss; the
  producing half is `localJournaled` and requires `approvedArmingDigest`. It changes
  `awaitingArm` to `armed` and is the sole source of the manual edit instruction.
- Manual support target mode: profile-bound choice between a bounded human
  window in the reserved original and an exact separate repository-bound
  working infobase; it is not selected ad hoc by the caller.
- Manual working-infobase lease: capability-proven non-human service lease held
  across final clean-state inspection and authorization terminalization in
  separate mode; busy/open/dirty state stops without automatic reset/discard.
- Manual original-infobase lease: equivalent exclusive configuration lease on
  the exact reserved original, acquired after the human closes Designer and held
  through support-action terminalization; repository root locking alone does not
  close the local-configuration race.
- Recovery retention provider: deny-unknown profile service that maps one stable
  provider object to an exact canonical recovery CF/SHA and owns idempotent task-
  scoped lease metadata only. Its capability denies rename/overwrite/delete
  through archive; Unica never mutates provider content.
- Manual support action: digest-bound pending authorization for one exact
  root-only transition set. `awaitingArm` permits only arming, cancellation, and
  status; `armed` additionally permits receipt-bound reconciliation. Before
  arming, the human may acquire only the root and must not edit or commit. The
  post-arm instruction asks the human to retain the root through the separate
  version, but acceptance is proven by exact actor/IB/delta, absence of an
  intervening root/support version, and the first such version after the arming
  cursor; release/reacquire without intervening root/support history is
  admissible. Stale at preview or apply final recheck leaves `awaitingArm` and
  requires a separate fully proven cancellation after release.
  An unknown effect in that pre-arm cancellation freezes only as
  `preArmCancellationEffect`: it has no arming receipt and requires separately
  approved observation and finalization recovery stages.
  The action must be reconciled, recovered, or cancelled with a complete history
  partition and closed manual lock/IB window before other authoritative work;
  cancellation may preserve classified non-action versions but cannot ignore a
  support/original/local delta.
- Support graph guard: configuration-root lock acquired before development
  objects so the ready support graph can be rechecked and frozen for main merge;
  it does not serialize unrelated non-root commits.
- Stale support preflight: phase entered when a gate-only mismatch is found with
  a root/full lock effect retained; the original remains unmutated and only
  exact verified unlock returns to `synchronized` for a new gate/plan.
- Support recovery disposition: one of `restoreThenReauthorize`,
  `preserveExternalAndReauthorize`, or `restoreThenAbandon`, bound to the full
  immutable history and exact recovery lock targets.
- Abandonment cleanup proposal: inverse root-only transition/evidence digest
  returned by archive `dryRun` preview without publishing an authorization or
  acquiring, inspecting, or releasing an external lease. Only a distinct
  approved archive apply may proceed; it journals before the lease gates and,
  after they pass, publishes the `awaitingArm` cleanup authorization.
  Missing/stale or capability-proven busy/dirty evidence stops typed, while an
  unknown lease effect is recovery.
- Abandonment ready: terminal precursor reached after inverse support cleanup;
  normally only status, classified routine refresh, and abandoned archive are
  legal. A current apply-published cleanup authorization narrowly adds arming/reconciliation/
  cancellation; a frozen one allows only status/recovery.
- Atomic commit safety evidence: capability-bound immediate pre-effect proof
  that post-merge history/reference/support drift cannot produce an accepted
  partial or stale task-content commit.
- Original infobase: developer infobase that remains connected to the main 1C
  configuration repository throughout branched development.
- Task infobase: owned disposable local File IB used for one task cycle.
- Branched task context: original project `cwd` plus opaque task/workspace IDs
  resolved internally to the owned disposable workspace by compatible tools.
- Unknown effect: a journal state where an external platform mutation may have
  occurred but its postcondition is not proven; only reconciliation may proceed.
- Domain event: a typed fact emitted by an operation, for example `FormChanged`.
- Reference operation script: Python/PowerShell/Bash donor model kept under
  `tests/fixtures` for parity tests. It is not a runtime backend.
- MCP: Model Context Protocol.
- Orchestrator: the Rust `unica` server that owns public tool dispatch and
  cache/state coordination.
- Public MCP server: the only MCP server visible to LLM through `.mcp.json`.
- Skill: Codex operation instruction under `plugins/unica/skills`.
- Workspace epoch: lightweight fingerprint used to associate cache state with
  the current workspace.
