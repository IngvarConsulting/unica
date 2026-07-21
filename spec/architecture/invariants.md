# Архитектурные инварианты

Этот документ фиксирует правила, которые должны оставаться верными при развитии
Unica. Если изменение нарушает инвариант, сначала нужен новый ADR, который явно
заменяет или уточняет текущее решение.

## Product Boundary

1. Unica is a Codex plugin for 1C:Enterprise developer workflows.
2. Public skills model developer operations, not infrastructure tools.
3. Low-level bundled tools must not become required LLM-visible knowledge.
4. The plugin must be usable from a generated marketplace package, not only from
   the source checkout.

## Public MCP Surface

1. The only public MCP server is `unica`.
2. `.mcp.json` must declare exactly one `mcpServers` entry.
3. `initialize` must return `serverInfo.name = "unica"`.
4. Public tools must use `unica.*` names.
5. Internal engines must not be exposed as separate MCP registrations.
6. Adding, removing, or renaming a public MCP tool requires tests and ADR sync.

## Skill Routing

1. Skills route through MCP `unica`.
2. Skills must not instruct the LLM to call internal adapter servers directly.
3. Skills must not use skill-local Python/PowerShell operation files as the
   target execution path.
4. Former script command semantics must be implemented inside native `unica.*`
   MCP tools. Reference scripts are allowed only under `tests/fixtures`.
5. For mutating operations with an honest preview, skills keep dry-run unless
   the user explicitly requested mutation. Operations without an honest preview
   use their declared closed-variant policy: `localJournaled` for owned local
   creation or an atomic task decision with no external effect, `contained`
   only for an actual owned probe/sandbox/evidence mutation,
   `preparedJournaledEffect` with its exact prepared/session/status digest, or
   `journaledEffect` with its exact guard or recovery digest. The latter two
   write intent before external effect and verify or reconcile the postcondition.
   They never fabricate a dry-run result; sandbox prepare/apply is limited to
   genuinely contained work.

## Application Boundary

1. Application use cases own tool dispatch and domain event emission.
2. MCP transport maps protocol requests to application calls.
3. Infrastructure adapters must not bypass application cache/event handling.
4. `unica-coder` must not contain a runtime operation-file fallback for
   Python/PowerShell/Bash scripts.

## Cache And Workspace State

1. The orchestrator owns workspace state and cache invalidation.
2. Mutating operations emit domain events for cache impact.
3. `UNICA_CACHE_DIR` can override the default volatile cache root.
4. Operations that support dry-run report cache impact without writing cache
   state.
5. Applied mutations may update `WorkspaceStateRepository`.

## Branched Development State And Safety

1. Configuration-repository workflow state is durable operational state, not
   volatile cache; it never depends on `.build/unica` or `UNICA_CACHE_DIR` for
   recovery.
2. Every effecting mutating branched-development call has a caller-stable
   `operationId`; replay with different canonical input is rejected. The
   strictly read-only `supportPrerequisiteArm` preview is the exception: it has
   no `operationId`, `dryRun`, or durable preview handle and is repeated after
   response loss; its apply is `localJournaled` and requires
   `approvedArmingDigest`.
3. The original infobase remains bound to the same repository and never receives
   task XML sources.
4. Baselines are verified full distributions. An ordinary CF is legal only in
   the registered `ordinaryResult` role and never substitutes for D0/Dn. CFU is
   classified only to reject it and is never a workflow input.
5. Supported update, main merge, repository mutation, rollback, and cleanup fail
   closed when the exact platform capability or postcondition is unproven.
6. Raw/public `-force`, supported-update/merge/commit/unlock force, implicit
   conflict defaults, automatic unresolved-reference clearing, and commit with
   `-keepLocked` are outside the automated workflow. The sole exception is the
   adapter-derived repository-update structural confirmation for an approved
   exact incoming add/delete plan with capability evidence.
7. Compensation releases only locks attributable to the current operation; an
   ambiguous or pre-existing lock is never released automatically.
8. Repository credentials are typed secrets and never enter public results,
   journals, retained logs, archives, or MCP stdout.
9. Cleanup targets only a marker-owned disposable instance root after canonical
   containment, nonce, symlink/reparse, Git/worktree, and terminal-state checks.
10. An implementation milestone cannot close issue #137 until the full
    modify/add/delete/reference/support/recovery acceptance matrix passes.
11. A non-overridable per-user target locator prevents state-root overrides
    from hiding unresolved tasks or start-attempt replay records.
12. Compatible general tools resolve an opaque `branchedTask` workspace ID;
    prompt-visible flows never pass the disposable path as `cwd`.
13. Repository-account identity has its own persistent reservation across
    different originals; exclusive same-user lock ownership is never shared by
    active tasks.
14. Any changed ordinary task mutation after verification atomically returns to
    `developing` and invalidates exactly every descendant workflow proof; a
    no-change mutation selects an equal before/result pair from the exact closed
    legal-phase union, preserves phase/evidence/cache, and replays identically;
    no other `TaskPhase` is representable. Both merge-resolution outcomes carry
    only `synchronizationConflicts -> synchronizationConflicts`.
    A changed manual-resolution receipt supersedes only prior selectable
    same-target receipts. If that target has a current conflict decision, the
    receipt moves its immutable revision to `replacementPending` without
    rewriting its consumed receipt; the next CAS-bound current decision names
    it through `replacesDecisionId`, and replay excludes the historical head.
    Other targets/decisions survive. All such receipts are session-scoped and
    expire with sandbox recreation.
15. Compatible general-tool results recursively project structured and free-text
    values to workspace-relative names or registered artifact IDs; absolute
    disposable/state/coordination paths and path-bearing secrets never cross
    the MCP boundary.
16. Main integration requires a current digest-bound no-force support preflight
    over the canonical UUID/ownership/reference candidate set; XML paths are not
    authoritative, CFU is rejected, and incomplete classification cannot report
    `ready`.
17. A human support prerequisite begins in `awaitingArm` and exposes only an
    instruction for the bound human actor to acquire the configuration root
    without editing or committing. The exact
    `repository.update(mode="supportPrerequisiteArm")` preview is strictly
    read-only, handle-free, and accepts no `operationId`/`dryRun`; only its
    `localJournaled` apply with `approvedArmingDigest` may change the action to
    `armed`. The pair proves the bound actor's current root ownership and a complete
    all-unrelated history prefix from the authorization cursor plus unchanged
    candidate set, relevant baseline, support graph, original fingerprint,
    recovery distributions, and retained handoffs, then durably binds an
    immutable arming receipt and edit
    instruction. A missing root returns acquire guidance and a root proven to a
    different actor returns release/coordination guidance; neither arms. Exact
    pre-arm drift never arms: stale at read-only preview or apply final recheck
    keeps `awaitingArm`, and neither stage cancels. Both require fresh preflight
    and request release iff the bound actor is still proven to hold the root;
    after release, only a separate cancellation with complete proof may publish
    its receipt. Incomplete evidence remains inconclusive. The instruction asks
    the human to retain the root, apply only the armed transitions,
    commit/release a separate version, and close Designer. Reconciliation is
    legal only from `armed` and accepts only the exact actor/IB and armed delta,
    with no intervening root/support version, as the first root/support version
    after the arming cursor. Release/reacquire without intervening root/support
    history is semantically admissible. The attributable version
    must use the profile-bound `reservedOriginal` or
    `separateWorkingInfobase` target. The authorization also binds a verified
    ordinary recovery distribution for every support layer reachable from the
    configuration-root support-settings window plus a task-scoped retention
    lease and receipt-proven profile-managed CF handoff readable by the bound
    manual actor; the deny-unknown provider binds one exact object/path/SHA,
    rejects rename/overwrite/delete through archive, and is never written,
    moved, quarantined, or deleted by Unica. CFU cannot satisfy that
    prerequisite. The separate mode additionally captures a clean repository-equal authorization
    baseline under an exclusive lease before exposing the instruction. The root
    must be released; reserved mode also restores its complete empty actor
    lock baseline. Accepted changes invalidate Dn-and-later evidence and force a
    fresh supported rebase.
18. Pending manual authorization is armed, reconciled, or explicitly cancelled
    only after a complete history partition beginning exactly at the
    authorization's expected-before cursor proves the applicable prefix and
    whether an attributable action version exists. Arming, reconciliation,
    cancellation, and frozen-recovery preview/apply retain that same lower bound
    and classify every version without a gap. Arm apply reproduces the approved
    all-unrelated partition and cursor exactly, without a suffix; armed frozen
    recovery also preserves the exact arming receipt. Cancellation supports
    both `awaitingArm` and `armed`, with receipt fields present only for the
    latter, and reconciliation also proves the manual lock/IB window is closed.
    A complete pre-arm root/support version is preserved as `preArmExternal`
    and never freezes as an armed violation. If cancellation from
    `awaitingArm` has an unknown guard/lease/update/terminalization effect, the
    authorization freezes only with `freezeKind=preArmCancellationEffect` and
    no arming receipt; observation and finalization are separately approved
    recovery stages. Finalization has one immutable intent/receipt per ordered
    effect: root acquire, mode acquire, optional update, cancellation, mode/root
    release, and terminal local receipt. Its under-guard recheck is the sole
    receiptless observation action; an already observed effect is never
    repeated. Replannable pre-update drift requires receipt-proven reverse
    compensation and an immutable attempt audit, while drift protected by a
    continuously held/update-ready guard is a capability breach. Known root/
    mode blockers are typed compensated stops whose full blocker evidence and
    external instruction remain digest-covered in the fresh current recovery
    plan for status reconstruction; unknown acquire/release remains
    recovery. The workflow is disabled before authorization unless a real
    fixture proves both guards survive worker/connector death until explicit
    release. Terminal status/archive preserve distinct cancellation/recovery
    receipt pairs plus the full finalization plan/prior audits, completed
    progress/full receipts, and complete effect/recheck/update/tail lineage. It
    can never acquire an armed recovery disposition.
    Classified non-action versions are preserved and only a classified
    external-support root is selectively applied during cancellation; routine
    tails wait for ordinary
    refresh. No support/original/local delta is ignored. Task-only support
    transitions are inversely restored before abandonment, the root guard is
    acquired/rechecked first, and successful
    original merge consumes its gate into receipt-bound historical evidence
    rather than staling it.
19. Every stale support gate identifies the exact changed input with complete
    expected/observed digests. Commit preview and immediate pre-effect recheck
    the consumed gate/merge/post-fingerprint lineage; drift enters exact
    restore/unlock recovery without starting a commit. An original-fingerprint
    mismatch is a retryable stale gate only with capability-proven clean
    out-of-band refresh evidence; an unowned/local/unknown delta enters recovery.
20. A global repository-history cursor is only an immutable scan position.
    Relevance is decided by a recomputed relevant-baseline digest and a complete
    contiguous semantic history partition; an unrelated cursor advance does not
    by itself invalidate Dn or become `relevantBaselineChanged`.
21. The configuration-root guard serializes root/support changes only. It does
    not claim to block unrelated non-root repository commits; every such advance
    across the observed apply/release interval is classified and retained explicitly.
22. Repository refresh acquires root first and the exact existing target/parent/
    referrer closure, applies the approved `-Objects` set, and verifies the
    resulting per-target revision/fingerprint map. A supposed
    `/ConfigurationRepositoryUpdateCfg -v` pin is never a safety primitive, and
    a whole-repository latest update cannot replace the selective plan.
23. If a support-gate-only mismatch is found after any root/full lock effect is
    retained, no original mutation starts and the task remains in
    `staleSupportPreflight` until exact verified unlock returns it to
    `synchronized`. A changed relevant baseline instead follows the fresh-Dn
    path through `staleRelevantBaseline`/`localVerified`.
24. Frozen support recovery locks only its digest-bound root/content/structural/
    reference targets and chooses exactly one disposition:
    `restoreThenReauthorize`, `preserveExternalAndReauthorize`, or
    `restoreThenAbandon`. External support history is never silently inverted;
    task-attributed unauthorized content or off-support history permanently
    forbids successful integration. A versionless `originalNotClean` restores
    only the classified repository baseline under `restoreThenReauthorize` and
    invents no repository version. Ownership may be reclassified only from a
    persisted invalid/unattributed observation with unchanged raw deltas; every
    positive or action-owned observation remains immutable.
25. Both manual target modes require the human to close Designer and a
    capability-proven service-held exclusive configuration lease on the exact
    affected IB through authorization consumption, cancellation, or frozen
    recovery terminalization: the original for `reservedOriginal`, the bound
    working IB for `separateWorkingInfobase`. A busy/open or dirty IB stops with
    typed external guidance after verified lease/guard release; Unica never
    resets or discards the human IB automatically. An unknown lease effect after
    authorization exists freezes armed `supportPrerequisite` lineage or, when
    awaiting-action cancellation was interrupted, the distinct no-arming
    `preArmSupportCancellation` lineage rather than creating generic lease
    recovery without the action/history binding.
26. Inverse-abandonment archive `dryRun` preview produces only a
    proposal/evidence digest, creates no authorization, and performs no external
    lease operation. The distinct approved archive apply journals before every
    lease gate and may publish `awaitingArm` only after they pass; known
    missing/stale or busy/dirty evidence stops typed with no authorization,
    while an unknown lease effect enters recovery. `abandonmentReady` then
    normally permits only status, classified routine refresh, and abandoned
    archive. A current cleanup authorization narrowly adds its arming/
    reconciliation/cancellation, while a frozen authorization permits only
    status and its exact recovery.
27. Final commit requires capability evidence for an atomic no-force safety
    boundary: the complete post-merge history/reference/support guard is
    rechecked immediately before effect. A pre-intent relevant tail, or a
    post-boundary locked-target/root change or deletion-blocking referrer, starts
    no accepted task-content commit; a capability-proven harmless closure
    expansion is recorded as `nonConflictingConcurrent`. Unproven/partial
    outcome enters recovery.
28. Retention-provider roots and their exact recovery sources are canonical,
    symlink/reparse-free, outside Git/root/home, and pairwise non-overlapping with
    work, instance, quarantine, original, repository, durable-state, and
    coordination roots in both containment directions. Start and every archive/
    cleanup destructive boundary revalidate this relation.
29. Retention-lease acquire/probe/release is idempotent and receipt-bound. A lost
    response replays or observes the same task-scoped lease; archive releases it
    exactly once only after durable handoff lineage, and an ambiguous release
    blocks cleanup. Provider operations may mutate lease metadata only.
30. Durable terminalization of a support authorization is irreversible. A
    contiguous post-release history tail may select only the already authorized
    safe result phase or create later gate/recovery evidence; it cannot reopen,
    reconsume, recancel, or otherwise mutate the terminal old authorization.

## Workspace Source Sets

1. Source format is a property of a source-set, not of the whole workspace.
2. One source-set must not be treated as mixed-format; conflicting format
   markers inside one source-set make it invalid/ambiguous.
3. A workspace may contain several source-sets with different effective
   formats, such as an EDT configuration and platform XML external processors.
4. Native platform XML metadata operations must select a platform XML source-set
   before editing XML files.

## Packaging

1. Generated binaries are not committed.
2. Packaged execution goes through checksum-verifying launchers.
3. The bundled public binary name is `unica`.
4. Generated package smoke must verify the packaged `.mcp.json`, not only source
   files.
5. Branched-development skills route only through public domain tools and must
   not expose Designer, raw repository flags, or `v8-runner` as workflow steps.
