# 6. Представление времени выполнения

## Initialize

1. Source checkout `.mcp.json` starts `cargo run --manifest-path ../../Cargo.toml --bin unica` from the plugin root.
2. Packaged `.mcp.json` invokes a command-scoped Git shell alias. The portable
   selector starts one native `unica-bootstrap` for the current host.
3. Bootstrap validates the pinned release manifest, obtains or reuses an atomic
   runtime cache, then replaces itself with or supervises `unica` while
   preserving stdio.
4. The Rust runtime resolver starts internal bundled tools directly from the
   cached `bin/<target>/<tool>` after SHA-256 verification.
5. MCP `initialize` returns `serverInfo.name = "unica"`.

## Tool List

1. MCP `tools/list` calls the application tool registry.
2. The response contains only `unica.*` tools.
3. Internal adapters are not listed.

## Concurrent MCP Dispatch and Cancellation

1. The stdio reader handles `initialize`, `tools/list`, and `ping` without
   waiting for an active `tools/call`; each tool call runs in its own worker.
   Input lines are capped at 8 MiB and only 32 tool workers are admitted.
   Oversized input returns `-32700`; saturation returns `-32603` with an
   `overloaded` message while synchronous `ping` and cancellation stay usable.
2. The dispatcher registers the JSON-RPC request ID with a cancellation token.
   Numeric and string IDs remain distinct.
3. `notifications/cancelled` with `params.requestId` cancels that token. The
   token is propagated through the application ports, CLI/index commands, and
   the workspace-service connector.
4. A cancelled request emits at most one response: JSON-RPC error `-32800` with
   message `request cancelled`. On EOF, already accepted workers get a 250 ms
   publication grace, then are cancelled and get up to 2 seconds to publish a
   terminal response. The publication-admission gate then closes without
   waiting for generic writer I/O: no response not already admitted may begin
   I/O after that linearization point. A publication already inside an arbitrary
   blocking `Write` may complete after `run_stdio_with_handler` returns; the real
   stdio process then exits and closes stdout. Response-writer failure also
   closes admission and cancels all active requests.

## Mutating Dry Run

This flow applies only when the operation has an honest, target-effect-free
preview. A no-preview mutation does not automatically use a sandbox:
`localJournaled` atomically records owned local creation or a task decision and
has no external effect; `contained` is reserved for actual owned
probe/sandbox/evidence work;
`preparedJournaledEffect` requires the exact prepared/session/status digest;
and `journaledEffect` requires the exact guard or recovery digest. The two
external-effect policies write intent before effect and verify or reconcile
postconditions. For example, supported-update preparation is
`contained`, while authoritative merge apply is `preparedJournaledEffect` and
repository lock/recovery effects are `journaledEffect`. The
`supportPrerequisiteArm` preview is a separate strictly read-only exception: it
accepts no `operationId` or `dryRun`, creates no durable preview handle, and is
repeated after response loss. Its effecting half is a distinct
`localJournaled` atomic apply with `operationId` and `approvedArmingDigest`, not
this `dryRun`/apply flow.

1. Caller invokes a preview-capable mutating tool without `dryRun: false`.
2. Application resolves `dryRun: true`.
3. Adapter returns the plan without changing target/workspace/cache/domain
   state. A durable workflow may persist only the bounded operation/idempotency
   and preview evidence needed to replay and authorize apply.
4. Application emits the relevant domain event for impact calculation.
5. Cache report returns `mode = "dry-run"` and impacted cache names.

## Applied Mutation

1. Caller explicitly passes `dryRun: false` for a preview-capable tool, or
   supplies the stable operation ID and any exact atomic-decision approval,
   prepared/session/status digest, or guard/recovery digest required by the
   closed variant.
2. Native MCP handler executes the declared policy, writing its required intent
   barrier before any external effect.
3. Successful mutation emits domain events.
4. `WorkspaceStateRepository` marks affected caches stale and records eager
   refreshes.
5. Result returns `{ ok, summary, changes, warnings, errors, artifacts, cache }`.

## Read Operation

Read tools do not emit mutation events by default. They may inspect current cache
state and, in future slices, trigger lazy refresh if a required cache is stale.

## Branched Development Cycle

1. `branched.status` reads existing durable state; `repository.recover` is the
   only operation that persists reconciliation after an interrupted effect.
2. `branched.start` validates the named local profile, task/operation IDs,
   capability row, state/work roots, exclusive original-infobase lease, and
   atomically reserved target plus dedicated repository-account reservation
   before creating a journal and owned
   instance. It also validates each deny-unknown retention provider, exact
   provider-object/source/SHA mapping, actor readability, lease capability, and
   that the source resolves below its provider root while that root remains
   disjoint from every owned/protected root. Both manual target modes validate
   their non-human service-inspection endpoint/secret references
   and exclusive-lease capability for the exact original or working IB; none of
   those paths, object handles, endpoints, or secrets cross the public result
   boundary.
   Local mutexes are host-local only. If either topology endpoint is multi-host,
   the row first proves the reachable linearizable shared coordinator and its
   fenced response-loss reservation receipts; otherwise start fails before task
   state exists.
3. Repository status and a digest-bound preview/apply update prove the
   still-bound original equals the approved repository object set. Apply uses
   root-first locking plus the exact existing target/parent/referrer closure,
   invokes the selective `-Objects` set, and verifies its per-target before/
   after revision/fingerprint map; it never relies on
   `/ConfigurationRepositoryUpdateCfg -v` as a version pin. Exact incoming
   add/delete may trigger only the capability-proven internal structural
   confirmation. Delivery creates and probes distribution D0, deploys a
   disposable File IB, and creates a guarded platform-XML Unica workspace.
   A baseline-role ordinary CF is rejected, while a registered ordinary-result
   CF remains the exact candidate-comparison/sandbox input. CFU is classified
   only to return its typed rejection and never enters the workflow.
4. Compatible existing typed tools receive the original `cwd` plus an opaque
   `branchedTask` context. The application resolves the owned task workspace,
   routes its cache events there, and durably receipts mutations without
   exposing its path. Changed ordinary mutations atomically return to
   `developing` and invalidate the exact descendant workflow closure; no-change
   receipts use a closed equal before/result phase pair and preserve phase/
   evidence/cache. Resolution receipts use only the equal
   `synchronizationConflicts` pair. Changed manual-resolution receipts supersede
   only older selectable receipts for the same target. If an earlier receipt
   was already consumed by current decision D1, it remains immutable/consumed;
   the edit atomically moves D1 to `replacementPending`, and the next CAS-bound
   decision D2 records `replacesDecisionId=D1`. Only current heads enter replay;
   session-scoped manual work is isolated from task sources. Final local verification freezes the IB/XML
   boundary, reruns configured checks, and records an immutable
   delta/checkpoint.
5. A current repository distribution D1 is applied first in a restored sandbox.
   Typed conflict decisions are replayed from the checkpoint; only a verified
   equivalent/adapted delta is repeated against the authoritative task IB.
6. Main integration first derives the complete canonical support candidate set
   and runs the real repository-fresh sandbox merge without force. A `ready`
   digest permits prevalidation and lock planning. A human-editable target stops
   with an `awaitingArm` authorization only after every root-reachable recovery
   CF has one task-scoped provider lease/receipt. The human first locks only the
   configuration root in the profile-bound `reservedOriginal` or
   `separateWorkingInfobase`, without editing or committing. A separate
   strictly read-only `repository.update(mode="supportPrerequisiteArm")`
   preview starts at the authorization cursor and proves a gap-free
   all-unrelated prefix, the exact bound actor's current root ownership, and
   unchanged candidate set, relevant baseline, support graph, original
   fingerprint, recovery set, and retained handoffs. It has no operation ID,
   `dryRun`, or durable handle and is repeated after response loss. Only the
   `localJournaled` apply with `approvedArmingDigest` moves the action to
   `armed` and durably publishes its receipt/edit instruction. A missing root returns acquire guidance and a
   proven wrong owner returns release/coordination guidance; neither arms.
   Exact pre-arm drift never arms: stale at preview or apply final recheck keeps
   `awaitingArm`; neither stage cancels. Both require fresh preflight and request
   release iff the bound actor still holds the root; after release, only a
   separate fully proven cancellation publishes the cancellation receipt.
   Missing evidence remains inconclusive. If that pre-arm cancellation has an
   unknown guard, mode-lease, selective-update, terminalization, or release
   effect, status exposes no-arming `preArmSupportCancellation` recovery:
   observe first, require approval of the new digest, then execute the exact
   one-effect/one-receipt sequence: missing root/mode acquisition, stage-sensitive
   recheck, optional selective update, authorization cancellation, missing mode/
   root release, and a distinct terminal local receipt. Already observed effects
   are never repeated. Replannable pre-update drift first compensates and archives
   the attempt; update-ready guarded drift is a capability breach. Known root/
   mode blockers stop typed with exact compensation and remain as a closed,
   digest-covered blocker/instruction in the fresh current recovery plan, so a
   status call reconstructs them after response loss; unknown effects remain
   recovery. A real capability row must prove both guards survive worker/
   connector death until explicit release. Terminal status/archive retain both
   receipt pairs, the full immutable finalization plan/prior audits, completed
   progress/full receipts, and complete effect/recheck/update/tail lineage. The edit instruction asks the human
   to retain the root, apply only the armed transitions, and commit/release a
   separate version. Reconciliation accepts by exact actor/IB/delta, absence of
   an intervening root/support version, and that version being first after the
   arming cursor; release/reacquire without an intervening version is admissible.
   The human then closes the bound Designer session and resumes through
   status. Reconciliation is legal only from `armed`, binds the arming receipt,
   proves every intervening version and the exact semantic delta, root freedom
   and reserved-mode actor-lock-baseline restoration, then returns to
   `localVerified` for fresh Dn/rebase/preflight. Cancellation is legal before
   or after arming and carries receipt evidence only after arming. Separate mode
   closes the terminal manual window under the working-IB lease; reserved mode
   holds the corresponding exclusive original-IB lease. Busy/open or dirty
   state releases every acquired guard/lease and stops for human cleanup,
   without automatic reset or authorization terminalization. An unknown
   separate-IB lease effect after authorization freezes armed
   `supportPrerequisite` recovery or awaiting-cancellation
   `preArmSupportCancellation` recovery, never an unbound generic lease plan.
   Invalid post-arm history uses recovery.
   Vendor-forbidden or inconclusive targets remain stopped.
7. Repository locks are acquired one object at a time with finite per-call
   deadlines, no polling, and intent-before-effect records. A failure compensates
   only observed operation-owned acquisitions; ambiguity enters
   `recoveryRequired`. The root guard is always acquired and the gate, including
   support absence, rechecked under it before any development-object lock. It
   serializes root/support changes only: unrelated non-root commits may advance
   the global history cursor and are accepted only through a complete contiguous
   semantic partition with an unchanged relevant-baseline digest.
8. Relevant anchors are checked after locking. A relevant change enters
   `staleRelevantBaseline`; a gate-only mismatch after a root/full lock effect
   enters `staleSupportPreflight`. Neither mutates the original, and only exact
   verified unlock can return to `localVerified` or `synchronized`
   respectively. An original-fingerprint change uses this retryable path only
   when a clean out-of-band refresh is capability-proven; otherwise it enters
   recovery. The original is otherwise merged only with the prepared
   settings, then maximum validation and exact result fingerprints run while
   locks are held. Frozen support recovery separately acquires only its
   digest-bound correction/finalization targets, preserves the same
   authorization-cursor history anchor, and completes exactly one of
   `restoreThenReauthorize`, `preserveExternalAndReauthorize`, or
   `restoreThenAbandon`; proven external support is never silently reversed. A
   versionless dirty original can use only the first disposition, and ownership
   evidence may reclassify only a prior invalid/unattributed observation without
   changing its raw deltas. A post-terminal tail may affect only the safe result
   phase or later evidence; it never reopens the old authorization.
9. One exact task-content integration-set commit, including add/delete entries
   and the frozen task-bound comment, is performed without force/keepLocked and
   accepted only after content and released task/support-guard lock proof. Its
   capability row must prove an immediate atomic no-force safety boundary over
   post-merge history/reference/support evidence: a pre-intent relevant tail,
   post-boundary locked-target/root or deletion-blocking-referrer conflict,
   intervening guard failure, or partial/unproven outcome publishes no accepted
   commit and enters recovery. Harmless post-boundary closure expansion is
   retained as `nonConflictingConcurrent`. Any earlier human root-only support prerequisite is separate
   archive evidence. Archive durably records every provider handoff/lease lineage
   before exact-once lease release; an ambiguous release remains recovery-bound.
   Cleanup protects the archived frozen canonical provider/source boundaries.
   After verified release it recanonicalizes a provider/source that still
   exists, but treats an external move/deletion as admissible absence; it never
   invents, traverses, or mutates provider content.
10. Safe abandonment requires original equality, no worker, no owned locks or
    pending/frozen support action, no unknown effect, and inverse reconciliation
    of every task-only support transition before archive/quarantine/cleanup.
    When inverse cleanup is needed, archive `dryRun` preview returns only the
    proposal/evidence digest, publishes no authorization, and performs no
    external lease acquire/inspection/release. The distinct approved archive
    apply journals before those lease gates; passing them publishes the
    `awaitingArm` authorization, missing/stale or proven busy/dirty evidence is
    a typed no-authorization stop, and an unknown lease effect enters recovery.
    That reconciliation reaches `abandonmentReady`; normally only status, a
    typed routine refresh for a concurrent repository advance, and abandoned
    archive remain. A current cleanup authorization narrowly permits its
    arming/reconciliation/cancellation, while a frozen one permits only status
    and its exact recovery.
    Successful original merge changes the ready support gate to receipt-bound
    historical evidence for post-merge verification/commit; that expected
    fingerprint change is not treated as staleness.

Each long `operationId`-bound contained or authoritative platform operation runs
in a dedicated worker that survives MCP stdio disconnect. Worker death leaves a
durable observation boundary and is never blindly restarted; only a
proven-contained owned-area outcome resumes from its recorded safe phase.
Read-only platform inspection is bounded, ephemeral, target-effect-free, and
safely rerunnable after its owned process and temporary output are discarded.
Task-workspace mutations report cache events against the disposable workspace
context, not only the original caller context.

## Workspace Analyzer Service

1. `unica.code.graph`, MCP-mode `unica.code.diagnostics`, and RLM-backed code
   navigation resolve the workspace and one effective source root. An explicit
   non-empty `sourceDir` is resolved relative to the request working directory.
   Without it, a source set named `main` wins; otherwise the sole
   `CONFIGURATION` source set wins. Missing or ambiguous choices fail with
   `invalid_source_root:` instead of silently using the workspace root.
2. The application asks the internal workspace service manager for a service
   keyed by normalized `workspaceRoot + sourceRoot`. The resolved source must
   remain inside the workspace and is also reported as the effective root by
   `project.status` and `project.map`.
3. If a matching live service exists, `unica` sends an internal localhost JSONL
   request using the token from `service.json`.
4. If the service is missing, stale, unreachable, or has a mismatched version,
   `unica` starts hidden mode `unica --workspace-service ...`.
5. The service keeps one persistent `bsl-analyzer` workspace MCP child and
   restarts it when source generation or explicit invalidation changes.
6. RLM index readiness/build/update is coordinated by the same service, but the
   RLM index remains a persistent file index under the workspace cache root.
7. Every analyzer or RLM work request carries a UUID `operation_id`. The shared
   runtime registers one cancellation token per operation and rejects duplicate
   IDs or new work after shutdown begins.
8. Accepted connections are handled independently. `ping`, `cancel`, and
   `shutdown` never acquire the analyzer lane and remain responsive while work
   is active. RLM jobs run outside that lane; only mutable access to the single
   warm analyzer session is serialized.
   The runtime caps general handlers at 64 and work workers at 8. Saturated
   general capacity feeds a bounded 64-socket, 500 ms aggregate control
   classifier (64 KiB classification prefix) and up to 8 reserved control
   handlers. It rejects classified work with the stable general-handler
   overload error and closes unclassified overflow; no unbounded thread or
   connection queue is created.
9. MCP cancellation causes the connector to send `cancel { operation_id }` on a
   separate connection. A disconnected work socket also cancels its operation.
   An operation guard removes the ID on every completion path, so the next call
    does not require a service restart.
10. Work and ordinary `Ping`, `Invalidate`, and `Shutdown` requests have one
    120-second overall deadline starting before connect. Control kinds use a
    500 ms connect cap; connect, write, flush, and read use the remaining
    overall budget. Reads poll every 100 ms so cancellation can be observed;
    cancellation takes precedence over timeout, EOF, protocol, and successful
    process-exit races. A best-effort `Cancel` is different: it uses a separate
    500 ms aggregate budget for connect, write, and flush and does not read a response.
    Internal request/response lines are capped at 8 MiB. Request-header parsing
    has one 5-second aggregate deadline from accept and polls in at most 100 ms
    slices; receiving another byte never resets the deadline.
11. Shutdown marks the runtime unavailable, cancels all active operations,
    rejects new work, removes the service record it owns, and drains handlers
    within the configured grace period.
12. Persistent analyzer and RLM subprocesses use `ManagedChild`. Windows starts
    each process suspended, assigns it to a kill-on-close Job Object, then
    resumes it; Unix creates a dedicated process group. Cancellation, timeout,
    and drop terminate the whole tree with bounded waits. Platforms other than
    Windows and Unix provide only immediate-child termination.

`initialize`, `tools/list`, `project.status`, `project.map`, `dryRun`, and
`unica.code.grep` do not start workspace analyzer services.

Bundled executable versions and assets are selected from
`plugins/unica/third-party/tools.lock.json`. CI validates the CLI/MCP surface of
the artifact selected by that lock rather than embedding a second analyzer
version constant.

## Bootstrap Cache Publication

1. The manifest must identify the exact source commit, `v<plugin-version>` tag,
   approved GitHub release origin, and all three supported targets.
2. A per-version/target lock serializes population under
   `$CODEX_HOME/unica/runtimes`.
3. Download and extraction occur in a UUID transaction directory on the same
   filesystem as the final cache.
4. Archive membership and SHA-256 hashes must exactly match the manifest.
5. `.ready.json` is written only after verification; the transaction is renamed
   atomically. Invalid prior state is quarantined and removed by the owning
   transaction.
6. `verify` performs MCP `initialize` and `tools/list` and requires the stable
   project/status and standards tools before reporting success.
