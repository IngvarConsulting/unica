# Architecture Change Checklist

Use this checklist when changing public MCP tools, skill routing, adapters,
cache behavior, or packaging metadata.

## MCP Surface

- [ ] `.mcp.json` still declares exactly one public server: `unica`.
- [ ] `initialize` still returns `serverInfo.name = "unica"`.
- [ ] `tools/list` contains intended `unica.*` tools only.
- [ ] Public tool name changes are covered by tests and ADR updates.

## Skill Routing

- [ ] Updated skills mention MCP `unica`.
- [ ] Updated skills do not expose internal adapter server names as user-facing
  routing.
- [ ] Updated skills do not point users to skill-local Python/PowerShell
  operation files.
- [ ] Skills use explicit `dryRun: false` for an ordinary honest preview/apply
  contract. A no-preview variant instead uses its declared policy:
  `localJournaled` for owned local creation or an atomic task decision with no
  external effect; `contained` only for actual probe/sandbox/evidence mutation;
  `preparedJournaledEffect` with the exact prepared/session/status digest, or
  `journaledEffect` with the exact guard or recovery digest. Both external-
  effect policies require an intent-before-effect barrier and postcondition
  verification/reconciliation. None fabricates `dryRun`; sandbox prepare/apply
  is used only for genuinely contained work. The `supportPrerequisiteArm`
  preview is the closed exception: it still requires `taskId`, accepts no
  `dryRun`/`operationId`, creates no durable preview handle, and is repeated
  after response loss; its apply is `localJournaled` and requires
  `approvedArmingDigest`.

## Cache And Events

- [ ] Mutating operation emits the right `DomainEventKind`.
- [ ] `CacheImpact` invalidates affected caches.
- [ ] Supported dry-run reports impact without target/workspace/cache/domain
  mutation; durable workflows may write only bounded idempotency/preview
  evidence. A no-preview mutation proves its policy-specific atomic-decision,
  prepared/session/status, or guard/recovery approval; only a `contained`
  variant uses an owned sandbox. No policy fabricates a preview.
- [ ] Applied operation writes remote-effect intent before mutation; it writes
  success/cache/domain-result state only after observed postconditions or an
  approved local state transition.
- [ ] Applied mutations notify live workspace services when analyzer or index
  caches are affected.

## Branched Development

- [ ] Durable task/operation records use `UNICA_STATE_DIR`/OS state, not the
  volatile workspace cache root.
- [ ] A non-overridable owner-only target locator prevents a state-root override
  from hiding unresolved tasks and preserves failed-start replay.
- [ ] Effecting mutating requests require stable `taskId` and `operationId`;
  replay input hashes and state transitions are covered by tests. The strictly
  read-only arm preview still has `taskId`, but omits `operationId` and
  `dryRun` and creates no operation/lease/start-attempt/receipt/durable-handle
  or task-status state. JSON-derived digest vectors use RFC 8785 JCS and
  domain-separated tool/policy input records; operation states are exactly
  `registered`, `intentWritten`, `effectUnknown`, and `terminal`.
- [ ] Compatible tools accept the original `cwd` plus opaque `branchedTask`,
  resolve the owned disposable workspace internally, and mutations return
  durable receipts/cache events for that context.
- [ ] Changed ordinary task mutations roll phase back to `developing` and
  invalidate the exact descendant closure atomically; schema/transition tests
  enumerate every legal changed/equal no-change phase pair, reject all other
  `TaskPhase` values, and require both merge-resolution outcomes to remain
  `synchronizationConflicts`. No-change preserves evidence/cache/replay.
  Changed merge-resolution receipts supersede only older selectable same-target
  receipts and remain session-bound. A fixture with consumed R1/current D1 then
  changed R2 proves R1 stays consumed/immutable, D1 becomes
  `replacementPending`, D2 carries `replacesDecisionId=D1`, operation replay is
  inert, other conflict heads survive, and resolved replay selects only D2.
- [ ] Compatible general-tool responses recursively project every structured and
  free-text field; byte/value scans prove that no absolute disposable,
  work-root, state, or coordination path crosses MCP.
- [ ] Designer argv is built from typed public/path/secret arguments and raw
  command material is absent from MCP responses.
- [ ] Repository update/acquisition/rollback/commit/unlock behavior is enabled
  only for a matching real-platform capability row; exact incoming add/delete
  is preview-bound before internal structural confirmation.
- [ ] Repository refresh uses the approved exact `-Objects` set and verifies
  per-target before/after revisions and fingerprints while root plus the exact
  existing target/parent/referrer closure is locked; no design or test relies on
  `/ConfigurationRepositoryUpdateCfg -v` to pin a repository version.
- [ ] Compensation touches only operation-owned locks; ambiguous ownership
  yields `recoveryRequired`.
- [ ] Original-target and repository-account reservations independently prevent
  concurrent tasks from confusing same-user lock ownership. Local mutexes/files
  are local only: any endpoint not capability-proven `hostConfined` uses the
  platform row's reachable linearizable coordinator for one atomic target-plus-
  account reservation, fenced response-loss reconciliation, and lease-loss
  persistence; unproven network-mounted files are multi-host and fail closed.
- [ ] Every recovery source resolves to its exact provider object below the
  canonical retention-provider root without traversal/symlink/reparse escape.
  Each provider root is disjoint from every work/instance/quarantine/original/
  repository/state/coordination root in both containment directions; start and
  archive/cleanup repeat Git, root/home, and nested-destructive-target guards.
- [ ] Support edits name one vendor layer and preserve unrelated layers byte for
  byte.
- [ ] Main-integration support preflight binds a complete canonical candidate
  set, no-force sandbox result, support graph, anchors, and capability row; XML
  paths/incomplete diagnostics cannot produce `ready`, and CFU is classified
  only for an evidence-bearing rejection.
- [ ] Global history-cursor advances are partitioned completely and relevance is
  decided from the task closure/relevant-baseline digest. The root guard is not
  treated as serialization of unrelated non-root commits.
- [ ] Gate-stale results return typed expected/observed input digests; root-first
  acquisition and pre-commit consumed-lineage drift have exact compensation or
  restore/unlock recovery evidence. A retained owned set uses the explicit
  `staleSupportPreflight` unlock exit rather than reusing a fresh-Dn state. An
  original-fingerprint mismatch is retryable only with a clean-refresh proof;
  unowned/local/unknown original state enters recovery.
- [ ] Manual support preparation enforces the profile-bound original/separate-IB
  mode, exact root-only transitions and root closure, an initial `awaitingArm`
  authorization whose only external instruction is to acquire the root without
  editing or committing, reserved-mode actor-lock baseline restoration, a
  verified ordinary recovery distribution for every
  support layer reachable from the configuration-root support-settings window
  plus its receipt-proven manual-actor-readable, profile-managed retained CF
  handoff (never CFU). The deny-unknown retention provider binds the exact object,
  canonical source and SHA, acquires/replays one task lease, rejects rename/
  overwrite/delete through archive, and permits Unica to mutate only lease
  metadata. Separate mode also captures a clean repository-equal authorization
  baseline under a released exclusive lease. Both modes require typed
  cancellation/recovery, inverse abandonment cleanup, separate archive evidence,
  and fresh Dn/rebase/preflight before lock planning.
- [ ] `repository.update(mode="supportPrerequisiteArm")` has a strictly
  `readOnly`, handle-free preview that requires `taskId`, has no
  `operationId`/`dryRun`, and has a separate `localJournaled` apply with
  `approvedArmingDigest`. The preview is repeated after response loss. Both
  start at the authorization cursor and prove
  a complete all-unrelated prefix plus unchanged candidate set, relevant
  baseline, support graph, original fingerprint, recovery distributions/
  handoffs, and the exact bound actor's current root ownership. Only apply
  changes `awaitingArm` to `armed` and
  publishes an immutable arming receipt/edit instruction; it must reproduce the
  approved partition/cursor exactly, without an appended suffix. A missing root
  returns acquire guidance; a proven wrong owner returns release/coordination guidance;
  neither arms. Exact pre-arm drift never arms: `stage=preview` and
  `stage=applyRecheck` both keep `awaitingArm`, and neither cancels. Both require
  fresh preflight and request release iff the bound actor still holds the root;
  after release only a separate `supportPrerequisiteCancellation` with complete
  proof may publish its cancellation receipt. Missing evidence remains
  inconclusive.
- [ ] Crash tests at every cancellation-from-`awaitingArm` guard/lease/update/
  terminalization/release barrier prove the frozen action has
  `freezeKind=preArmCancellationEffect`, no arming receipt/disposition, an
  observation-only first recovery, a digest-changing reapproval stop, and only
  then exact one-effect/one-receipt finalization. The capability fixture proves
  root/mode guards survive worker/connector death until explicit release; tests
  cover stage-sensitive recheck, compensated/audited pre-update replan,
  update-ready protected-boundary breach, known root/mode blockers, unknown
  acquire/release, durable status reconstruction of a known blocker's exact
  evidence/instruction, and terminal status/archive equality for the full
  finalization plan/prior audits/completed receipt progress, distinct
  cancellation/recovery receipts, and update/tail lineage.
- [ ] After arming, the instruction asks the human to retain the root, apply only
  the receipt-bound transitions, commit/release a separate version, then close
  Designer and resume through status/reconciliation. Acceptance proves the
  exact actor/IB and armed delta, no intervening root/support version, and that
  the attributable version is first after the arming cursor; release/reacquire
  without intervening root/support history is admissible. Reconciliation requires the exact
  arming receipt; cancellation covers `awaitingArm` and `armed` and carries
  receipt fields only for the latter.
- [ ] Both manual modes instruct the human to close Designer and validate the
  non-human inspection/exclusive-lease capability for the exact IB. Reserved
  mode leases the original and separate mode the bound working IB through
  consume/cancel/frozen terminalization; busy/open, dirty, and unknown fixtures
  prove no automatic reset, discard, unproven authorization terminalization, or
  orphaned lease/guard.
- [ ] Arm/prerequisite/cancellation previews, their apply evidence, and every
  frozen support-recovery partition begin exactly at the authorization's
  expected-before history cursor and contain every intervening version. Tests
  reject a shifted lower bound, omitted prefix, gap, substituted arming receipt,
  root/support version inserted before the authorized version, or mismatched
  approved range.
- [ ] Frozen support recovery carries complete version observations, exact
  correction/finalization lock targets, and one of the three dispositions;
  proven external support is preserved and task-attributed content/off-support
  taint can reach only abandonment. A versionless `originalNotClean` uses only
  `restoreThenReauthorize`; ownership reclassification is legal only for a prior
  invalid/unattributed observation with unchanged raw deltas; terminal
  post-release tails never reopen the old authorization. Tests cover
  `DeferredRepositoryAdvance` as `classified`, `unclassified`, and
  `coverageUnknown` without an invented version; incomplete and complete
  routine previews preserve the handle/phase, while only the verified approved
  apply consumes the exact `observationDigest` and selects the safe relevant
  phase.
- [ ] An unknown separate-working-IB lease effect after support authorization
  exists freezes the armed `supportPrerequisite` plan or, during awaiting-action
  cancellation, the no-arming `preArmSupportCancellation` plan;
  generic `manualWorkingInfobaseLease` recovery is limited to pre-authorization
  inspection and cannot orphan a pending action.
- [ ] Retention tests resolve `retentionCapabilityRowId` only through the
  tracked closed retention-provider manifest and keep the separate
  `manualReadabilityCapabilityRowId` on its exact platform row/case. They cover
  missing/stale/mismatched/unpassed rows, skipped or digest-mismatched cases,
  acquire response loss/idempotent replay, held observation, exact-object/SHA/
  readability revalidation, rename/overwrite/delete denial, unknown-effect
  reconciliation, path/symlink/reparse containment, durable archive-before-
  release, exact-once release replay, ambiguous release blocking cleanup,
  still-live identity revalidation, and successful cleanup after an external
  post-release source move/delete without touching provider content.
- [ ] Inverse-abandonment archive tests prove that `dryRun` preview returns only
  a proposal/evidence digest and performs no external lease operation; the
  distinct approved apply journals before lease gates and publishes
  `awaitingArm` only after success. Missing/stale and proven busy/dirty evidence
  stop typed with no authorization, while unknown lease effects enter recovery.
  `abandonmentReady` tests then cover its normal status/routine/archive surface,
  the narrow pending cleanup arming/reconciliation/cancellation exception, and
  the frozen status/recovery-only exception.
- [ ] Commit capability fixtures prove the immediate post-merge history/
  reference/support guard and atomic no-force failure boundary; a pre-intent
  relevant tail or post-boundary locked-target/root or deletion-blocking-referrer
  conflict cannot publish commit success, while harmless expansion is retained
  as `nonConflictingConcurrent`. Partial/unproven results remain recovery.
- [ ] Cleanup revalidates the owned marker, nonce, containment, and every
  symlink/reparse/Git/root guard immediately before quarantine and deletion.
- [ ] Package and real-platform evidence cover the full acceptance matrix in
  `spec/acceptance/branched-development.md`.

## Adapters

- [ ] Internal adapter errors are summarized in `warnings` or `errors`.
- [ ] Adapter command construction is covered by focused tests when behavior is
  non-trivial.
- [ ] Analyzer/index adapters that need warm workspace state go through the
  workspace service manager.
- [ ] Cheap read-only adapters such as `unica.code.grep` do not start workspace
  services.
- [ ] Operation backends use native Rust handlers, not Python/PowerShell/Bash
  runtime fallbacks.
- [ ] Fixture parity exists when donor script behavior is retained as the
  reference source model.

## Packaging

- [ ] `third-party/tools.lock.json` names the bundled binary `unica`.
- [ ] Generated `third-party/manifest.json` matches the lock.
- [ ] `cargo run --quiet --bin unica -- --help` works from source checkout.
- [ ] Generated package `.mcp.json` starts `./bin/<target>/unica` directly with
      `cwd` set to the plugin root.
- [ ] Fresh Codex visibility is checked from a clean cache when changing plugin
  metadata.

## Verification

Run:

```sh
cargo fmt --all -- --check
cargo clippy --package unica-coder --all-targets -- -D warnings
cargo test --package unica-coder
python3.12 -m unittest discover -s tests/ci
git diff --check
```

BSP parity fixtures are the narrow exception to whitespace normalization: they
preserve harvested bytes under `.gitattributes` `-text -whitespace`, and their
manifest hashes are the integrity check.
