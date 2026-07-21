# 11. Риски и технический долг

## Active Risks

- Standards adapter is not yet a full native HTTP MCP proxy.
- Native XML/DSL handlers can drift from donor behavior if parity fixtures are
  not updated together with Rust ports.
- Cache reporting exists before full lazy/eager rebuild implementation.
- The public tool list can grow too broad if every internal capability is
  mirrored one-to-one.
- Fresh Codex visibility can be affected by stale local plugin cache.
- Designer batch documentation does not prove atomic locks/commits, global lock
  ownership discovery, or rollback semantics required by branched development.
- Existing support editing is not layer-aware and cannot preserve arbitrary
  multi-vendor chains for this workflow.
- Designer credentials supplied as argv may be visible to privileged local
  process inspection even when Unica logs and state are fully redacted.
- Capability evidence can drift across platform version, OS, locale, and
  encoding; version detection alone is not a safety proof.
- A repository-history cursor can advance because of unrelated work; treating it
  as a relevant baseline would cause needless rebases, while assuming the root
  guard blocks non-root commits would silently lose concurrency evidence.
- `/ConfigurationRepositoryUpdateCfg -v` does not provide the required pinned
  update semantics; a broad latest update can ingest changes outside the
  approved plan.
- A separate human working IB can remain open, busy, or locally dirty after its
  root commit, so a snapshot without an exclusive service lease can orphan an
  uncommitted support change.
- A pre-arm cancellation crash between selective original update and durable
  authorization cancellation is unsafe if either the repository-root capture or
  mode lease disappears with its worker/connector; a journal claim alone cannot
  reconstruct the protected boundary.
- The reserved original has the same local-configuration race: a repository
  root lock and empty actor-lock inventory do not prevent an open Designer from
  changing the IB between fingerprint inspection and authorization
  terminalization.
- A one-step manual-support instruction can authorize editing after preflight
  even though repository history, the support graph, original fingerprint, or a
  retained recovery handoff changed before the human obtained the root. A root
  acquired by the wrong actor, an intervening root/support version, or inability
  to prove the exact actor/IB/delta and first-version ordering destroys the
  attribution boundary. Release/reacquire by itself does not: it remains
  admissible when no intervening root/support version exists and the complete
  attribution evidence is still provable.
- A retention-provider profile can point a recovery source/provider root into an
  owned, quarantine, original, repository, state, or coordination tree. A lost
  acquire/release response or a provider that permits overwrite/delete can make
  recovery unreachable or turn cleanup into external-content deletion.
- A support history partition whose lower bound is not the authorization cursor
  can omit an early authorized/conflicting version. Over-broad ownership
  reclassification or treating a post-terminal tail as a new action can then
  rewrite immutable recovery evidence.
- A final commit implementation without a proven immediate atomic no-force
  safety boundary can race with a new relevant/referrer repository version or
  report partial success.
- Per-user coordination files and OS mutexes cannot exclude a second host that
  reaches the same original or reserved repository account. Treating an
  unproven network-mounted file endpoint as local would admit conflicting tasks.
- A fifth durable `observed` operation state or a read-only durable handle would
  make replay/status semantics ambiguous and could convert a target-effect-free
  inspection into recoverable state.

## Mitigations

- Keep gaps in the implementation task list.
- Add parity fixtures and MCP contract tests for donor behavior that must remain
  compatible.
- Keep `.mcp.json` single-server tests.
- Validate generated marketplace packages, not only the source checkout.
- Use clean `CODEX_HOME` for visibility proof.
- Gate repository automation on exact retained real-platform capability rows;
  keep unproven fields nullable and unproven mutations disabled.
- Record endpoint reachability in that same platform row and, whenever either
  endpoint is multi-host, require a reachable linearizable coordinator with
  atomic target/account reservations, fenced idempotent receipt reconciliation,
  two-host contention, and response-loss fixture evidence. Do not add a third
  manifest or treat local mutexes as that proof.
- Use a dedicated integration account, durable operation journal, exact
  compensation, and no-force unlock/recovery rules; gate the distinct
  repository-update add/delete confirmation on an approved plan and fixture.
- Require layer-aware support round trips and full packaged acceptance before
  closing issue #137.
- Disable manual-support authorization unless a real crash fixture proves the
  root capture and selected mode lease survive worker/connector death until
  explicit receipt-bound release. Give every finalization effect one immutable
  intent/receipt, audit compensated pre-update replans, and treat protected-
  boundary drift as a capability breach rather than a retry.
- Treat target support readiness as a four-outcome, no-force, capability-proven
  gate. Publish only an acquire-root-without-edit instruction, then use a
  strictly read-only `supportPrerequisiteArm` preview with no `operationId`,
  `dryRun`, or durable handle to recheck the complete authorization-anchored
  unrelated prefix, actor-owned root, candidate/relevant baseline,
  support/original, and recovery handoffs. Repeat that observation after
  response loss; only its `localJournaled` apply with `approvedArmingDigest`
  may publish the immutable receipt that permits editing. Stale at preview or
  apply final recheck leaves `awaitingArm`, neither arms nor cancels, and after
  root release requires a separate fully proven cancellation; both routes
  require fresh preflight. Accept a manual support change only
  in its profile-bound target mode as the exact actor/IB/delta and first
  root/support version after the arming cursor with no intervening root/support
  version. The instruction still asks the actor to retain the root, but
  release/reacquire without intervening history is admissible while root
  closure, exact actor/IB/delta/first-version evidence, and reserved-mode
  actor-lock closure remain provable; type
  cancellation/recovery/inverse cleanup, then invalidate and rebuild the
  complete Dn lineage.
- Keep global cursor, relevant-baseline digest, and complete semantic history
  partition as separate evidence. Test unrelated commits during every root-guard
  window instead of claiming global serialization.
- Use an approved selective `-Objects` update with per-target revision/
  fingerprint checks; never advertise `-v` as a version pin. Preserve exact
  structural confirmation and recovery lock closures behind capability rows.
- Require a non-human inspection endpoint and exclusive configuration lease for
  both manual targets: the exact original in reserved mode and the exact working
  IB in separate mode. Instructions require Designer closure; busy/dirty fixtures
  prove guard/lease release and absence of automatic reset or authorization
  finalization, while an unknown post-authorization lease effect remains bound
  to the armed support-prerequisite recovery or the distinct no-arming
  pre-cancellation recovery, according to the interrupted action state.
- Validate a deny-unknown retention-provider registry, exact provider-object/
  source/SHA mapping below its canonical provider root, and provider-root versus
  owned/protected-root non-overlap before task creation and every destructive
  boundary. Prove idempotent acquire/replay/release, WORM rename/
  overwrite/delete denial, archive-before-release, and cleanup refusal on an
  ambiguous release.
- Anchor prerequisite/cancellation/frozen history at the authorization cursor;
  permit ownership reclassification only from prior invalid/unattributed
  evidence with unchanged raw deltas, model versionless `originalNotClean`
  without a fake version, and keep terminal authorizations immutable across
  post-release tails.
- Gate final commit on retained real-platform evidence for the immediate atomic
  no-force safety check, including relevant-tail and partial-result failures.
