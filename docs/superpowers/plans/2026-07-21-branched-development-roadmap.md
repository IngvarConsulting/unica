# Branched Development Delivery Roadmap

> **For agentic workers:** REQUIRED SUB-SKILL: Use
> `superpowers:subagent-driven-development` to execute each bounded task with a
> fresh implementer and reviewer. Use `superpowers:test-driven-development` for
> every behavior change and `superpowers:verification-before-completion` before
> any completion claim.

**Goal:** Deliver the complete ADR-0012 lifecycle behind the single public
`unica` MCP server, with all 21 typed `unica.*` tools, durable replay and
recovery, package contracts, and the acceptance evidence required by issue
#137.

**Approved baseline:** Specification revision `1531d36`, approved by the owner
on 2026-07-21. Code and tests remain the higher source of truth; any discovered
contradiction must stop the affected slice and be resolved in the ADR/contracts
before implementation continues.

**Architecture:** The new `domain::branched_development` bounded context owns
closed workflow types and invariants. Application services orchestrate ports;
infrastructure adapters own durable state, coordination, platform execution,
archive, and package integration. Legacy `WorkspaceStateRepository`,
`.build/unica`, `UNICA_CACHE_DIR`, runtime jobs, raw Designer commands, and raw
`v8-runner` tools are not workflow backends or prompt-visible fallbacks.

## Non-negotiable gates

- Keep one MCP server named `unica`; expose only typed `unica.*` tools.
- Do not register any of the 21 handlers until request/result Rust types,
  strict generated schemas, committed snapshots, policy metadata, and source
  plus packaged contract tests exist for all 21 tools.
- Preserve the serialized legacy `OperationResult` envelope and existing
  fields. Literal Rust struct construction cannot remain source-compatible when
  fields are added, so record and test that deliberate API expansion instead
  of making an impossible claim. Enforce workflow presence rules internally
  with a closed result enum, then project into typed optional workflow fields.
  Every task-bound result still carries
  `changes`, `artifacts`, and `cache`; branched results expose no command,
  executable path, stdout, stderr, credential, or task filesystem path.
- `readOnly` is never a durable operation policy. Durable records accept only
  the five effect-capable policies and reject a legacy/stored `readOnly` record
  as `stateCorrupt` before CAS, lease, worker, receipt, replay, or effect.
- Operation input hashing uses RFC 8785 JCS and domain separation. It excludes
  only the top-level `operationId`; duplicate JSON member names are rejected at
  the protocol boundary before hashing.
- Operation state is exactly `registered`, `intentWritten`, `effectUnknown`, or
  `terminal`. `observed` is an fsynced barrier, never a state.
- Read-only status performs no lazy migration, cache initialization, locator
  creation, CAS, lease, receipt, worker, or external effect.
- Multi-host and manual-support paths fail closed until their exact coordinator,
  retention, actor, artifact, and capability evidence is both specified and
  certified. Empty capability rows are safer than inferred support.
- A vertical slice is a milestone, not completion. Completion requires modify,
  add, delete, form/attribute ownership, reference closure, multi-vendor support
  isolation, interruption recovery, archive/cleanup, package verification, and
  the full acceptance matrix.

## Delivery sequence

### Phase 0 — approved baseline and operation kernel

- [x] Rebase the reviewed identifier/lifecycle/replay kernel onto `1531d36`.
- [x] Re-run the full workspace baseline and focused boundary checks.
- [x] Complete durable-policy, strict JSON, JCS digest, and replay-kernel repair.
- [x] Complete the exact-byte forbidden-read-only poison preflight without
  inventing the not-yet-generated operation-record schema.

Exit: the pure kernel cannot represent durable `readOnly`, ambiguous canonical
input, or an illegal operation state; exact bytes carrying a stored top-level
`readOnly` are retained and rejected before any full-record candidate. Complete
`stateCorrupt` mapping waits for the real generated storage schema in Phase 2.

### Phase 1 — executable contract for all 21 tools

- [ ] Define the complete closed tool-name, request, result-data, stop-data,
  rejection-data, stable-error, phase, action, instruction, and receipt types.
- [ ] Extend `OperationResult` source-compatibly and introduce an internal
  `BranchedToolResult` that makes completed/stopped/rejected presence rules
  unrepresentable.
- [ ] Generate strict JSON Schemas and policy metadata from Rust types.
- [ ] Define and snapshot the complete operation-record, lease,
  `ActiveOperationStatus`, terminal-envelope, and storage framing schemas,
  including how the current schema digest is selected without inventing an
  embedded field absent from ADR-0012.
- [ ] Commit source snapshots for all 21 tools plus a generated package manifest
  of their exact digests; after handler registration, built-package
  `tools/list` is compared against those same source fixtures.
- [ ] Prove exact argument sets, `additionalProperties: false`, required fields,
  policy mappings, stable codes, and absence of raw-path/process fields.

Exit: all 21 contracts exist and agree before the first handler is registered.

### Phase 2 — durable state, coordination, and replay

- [ ] Implement owner-only state roots, non-overridable locators, schema-digest
  validation, atomic write/fsync/rename, and retained corrupt bytes.
- [ ] Complete Task 5B: full typed operation-record loader and deterministic
  `stateCorrupt` evidence from committed expected schema digest plus exact-byte
  observed digest, before any status/replay/CAS/effect path.
- [ ] Implement task records, start-attempt records, operation records, leases,
  heartbeats, receipts, effect journals, result envelopes, and gate history.
- [ ] Implement persistent target/account reservations with atomic admission and
  exact-key busy results.
- [ ] Implement one-writer CAS, generation fencing, owner liveness, response-loss
  replay, orphan recovery, and the complete crash matrix.
- [ ] Prove state-root overrides cannot hide an unresolved task/start attempt.

Exit: every durable barrier is replayable or explicitly recovery-required, with
no duplicate effect possible under supported topology.

### Phase 3 — profile, capability, start, and status

- [ ] Parse and validate the versioned profile, platform rows, topology rows,
  repository transport, database mode, tool variants, secret references,
  retention provider, and general-writer capability manifest.
- [ ] Fail closed for missing, stale, ambiguous, or uncertified capability rows.
- [ ] Implement internal full `branched.status` projection for `notCreated` and
  every existing-task field without writes. Active operation status projects
  only registered/intent-written/effect-unknown validated records; terminal
  results use their separate evidence projection.
- [ ] Implement `branched.start` preflight, canonical identities, task/admission
  reservations, owned roots, disposable File IB, D0, and initial durable state.
- [ ] Register status/start only after Phase 1's all-tool contract gate passes.

Exit: status is a complete observational projection and start is idempotent,
reserved, fail-closed, and leaves no hidden partial task.

### Phase 4 — owned workspace and authoring bridge

- [ ] Implement owned task/probe/merge workspace services and safe path policy.
- [ ] Add durable event/hash receipts for every compatible general writer.
- [ ] Replace the broad support edit behavior with exact typed layer/root
  operations required by ADR-0012; never reset unrelated support flags.
- [ ] Prove incompatible or absent BSL writers disable only BSL-dependent tasks.

Exit: task-local edits are attributable, hash-bound, and cannot escape the
owned workspace or silently mutate unrelated support settings.

### Phase 5 — platform and artifact ports

- [ ] Define `DesignerPort`, repository, distribution, database, filesystem,
  clock, process, archive, retention, and capability ports.
- [ ] Build deterministic fakes for every success, localized diagnostic,
  timeout, crash, partial effect, unknown effect, and response-loss branch.
- [ ] Implement guarded platform adapters without exposing raw commands publicly.
- [ ] Validate CF versus CFU identity and immutable SHA/size/provenance evidence.

Exit: orchestration is fully testable without a live platform and all live
effects pass through narrow auditable ports.

### Phase 6 — delivery lifecycle

- [ ] Implement `delivery.inspect`, `create`, `verify`, and `deploy`.
- [ ] Create D0/D1 with `/CreateDistributionFiles -cffile`, verify ordinary CF,
  and reject CFU everywhere.
- [ ] Prove disposable-IB leases, clean state, immutable artifact lineage,
  response-loss replay, and cleanup ownership.

Exit: distribution artifacts are verified, immutable, task-bound, and safe to
resume after interruption.

### Phase 7 — compare, conflict, supported merge, and verification

- [ ] Implement `merge.compare`, `prepare`, `conflicts`, `resolve`, `apply`, and
  `verify` as supported three-way D0/D1/local operations.
- [ ] Before registering `merge.prepare` or `merge.resolve`, freeze the exact
  platform conflict classifier with fake and real fixtures. It must emit each
  conflict's non-empty canonical `allowedResolutions`; no static seven-by-four
  matrix may be inferred from enum names, and every non-member pair is rejected
  before receipt or decision mutation.
- [ ] Persist exact conflict decisions and adapted/equivalent delta evidence.
- [ ] Prove no ordinary XML load touches the original repository-bound IB.
- [ ] Run local validation and supported-update postconditions at every barrier.

Exit: local development reaches a fully explained synchronized result with no
unattributed or unexpected delta.

### Phase 8 — repository observation, preflight, and selective update

- [ ] Implement read-only `repository.status` with completeness and nullable
  owner evidence; presentation text never drives identity or control flow.
- [ ] Derive the full UUID/ownership/reference candidate closure.
- [ ] Implement the four-outcome main support preflight in a repository-fresh
  disposable sandbox without force or CFU.
- [ ] Implement exact preview/apply `repository.update`, selective target guards,
  stale-plan rejection, compensation, and post-update proof.

Exit: main integration is supported, candidate-complete, preview-bound, and
fails closed on unknown ownership, references, diagnostics, or support state.

### Phase 9 — manual support prerequisite

- [ ] Implement reserved-original and separate-working-IB authorization modes,
  exact root/layer distributions, immutable handoff, retention, and leases.
- [ ] Implement repeatable read-only arm preview and separate digest-approved
  `localJournaled` arm apply with final recheck.
- [ ] Implement exact actor/root/version acceptance, stale-data cancellation,
  frozen recovery, all three dispositions, and history-partition continuity.
- [ ] Keep this path unavailable until exact coordinator/retention/manual-case
  contracts and certified capability rows remove the identified ambiguities.

Exit: no human edit is authorized without a complete, retained, digest-bound,
recoverable handoff and exact evidence chain.

### Phase 10 — lock window and bounded main integration

- [ ] Implement `repository.planLocks`, `lock`, and `unlock` with root-first
  stable ordering, exact target identities, nullable owners, and compensation.
- [ ] Recheck binding, support, baseline, candidate closure, and approvals under
  acquired guards before any merge.
- [ ] Bound merge/validation time and persist every intent, observation, receipt,
  blocker, and release outcome.

Exit: the short main lock window never expands silently and unknown acquisition,
merge, validation, or release effects require explicit recovery.

### Phase 11 — commit and recovery

- [ ] Implement exactly one task-content `repository.commit` with release.
- [ ] Implement `repository.recover` for every declared unknown-effect,
  committed-unverified, lock compensation, support conflict, cancellation, and
  digest-changing reapproval branch.
- [ ] Prove recovery never guesses repository version, ownership, or effect.
- [ ] Prove replay reconstructs typed terminal/stopped/rejected results after
  response loss without repeating an effect.

Exit: every interruption point has a deterministic observe/reapprove/recover
path and no unsafe automatic inverse.

### Phase 12 — archive and cleanup

- [ ] Implement `branched.archive` with complete immutable lineage and retention
  evidence before any destructive cleanup.
- [ ] Implement `branched.cleanup` using only verified ownership tokens and
  explicit safe roots; never reconstruct a target from untrusted input.
- [ ] Prove external moves/deletes, ambiguous ownership, missing archive,
  unresolved effects, and active leases block or safely classify cleanup.
- [ ] Release every task/reservation/retention lease exactly once.

Exit: successful and abandoned tasks terminate with verified archive lineage and
only owned, no-longer-needed material is removed.

### Phase 13 — package and prompt-visible workflow

- [ ] Add the MCP-first `unica:branched-development` skill using only typed
  `unica.*` tools and documented utility exceptions.
- [ ] Update plugin metadata, `.mcp.json`, tool lock, generated package, schema,
  policy, provenance, and package smoke fixtures together.
- [ ] Verify built/installed artifact `tools/list` and representative calls, not
  merely source registration.
- [ ] Keep release notes and README aligned with proven capability rows only.

Exit: source and packaged behavior are identical and no raw Designer/script
fallback is prompt-visible.

### Phase 14 — certification, final audit, and PR readiness

- [ ] Run pure/unit/property/concurrency/crash/package suites for the full matrix.
- [ ] Run real 1C fixtures for distribution, supported update, repository history,
  target/owner diagnostics, lock/commit/release, multi-vendor support, database
  topologies, manual modes, and restart barriers.
- [ ] Run two-host races where certified; otherwise keep multi-host capability
  rows empty and document the unimplemented certification, not a false claim.
- [ ] Perform whole-branch source-of-truth audit, security review, code review,
  formatting/lints/full tests, installed-package verification, and CI follow-up.
- [ ] Update PR #173 from draft only after every required completion criterion is
  proven or the approved scope is explicitly amended by the owner.

Exit: PR #173 is reviewable as a working, fail-closed implementation whose
claims exactly match its executable evidence.
