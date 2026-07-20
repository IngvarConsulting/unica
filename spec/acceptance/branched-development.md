# Branched Development Acceptance

## Goal

Prove that an installed Unica package can complete a development task against a
main 1C configuration repository while the original infobase remains bound,
all development happens in an owned disposable branch, integration is scoped
and recoverable, and cleanup occurs only after a proven terminal outcome.

This matrix is normative for [issue #137](https://github.com/IngvarConsulting/unica/issues/137).
A milestone, mock-only implementation, or happy-path demonstration cannot close
the issue while any row lacks the stated evidence.

Exact request/response schemas, execution policies, lock mapping, and stable
errors are normative in the
[Branched Development Tool Contract](../contracts/branched-development-tools.md).

## Requirement Matrix

| ID | Requirement | Required evidence |
| --- | --- | --- |
| BD-01 | Original infobase remains bound to the same repository; XML is never loaded into it | Real fixture records binding identity before/after every phase and a Designer invocation transcript with no original-target XML load |
| BD-02 | D0 and every refresh Dn are verified full distributions; result CF is ordinary; CFU is rejected | Artifact probe tests plus real create/deploy fixture and strict tool schemas |
| BD-03 | Each task gets a new owned File IB/XML workspace and cannot reuse a stale baseline | Task-state/path unit tests, packaged second-cycle E2E, marker and instance-ID evidence |
| BD-04 | Only actually changed objects become editable in the exact support layer; unchanged objects relock, new vendor objects are non-editable, all other modes and upstream chains survive | Layer-aware parser round-trip tests and real unsupported/multi-vendor fixtures |
| BD-05 | Synchronization is a true L1+D0+D1 supported update before repository locking | Invocation/evidence order and real twice-changed property fixtures |
| BD-06 | Pre/post canonical project delta is `equivalent` or explicitly `adapted`; missing/extra changes stop | Pure delta tests plus fake and real supported-update scenarios |
| BD-07 | Every scalar/module/add-add/delete-modify/reference/support conflict is typed and explicitly resolved | Settings/parser fixtures, decision journal, sandbox replay and real platform cases |
| BD-08 | Added objects preserve UUIDs; lock plans cover modify/add/delete/forms/attributes/references with a reason per object | Ownership/reference graph unit tests and real integration scenarios |
| BD-09 | A foreign lock stops integration within finite per-call and whole-transaction deadlines; no polling occurs, and every lock acquired by that operation is released and verified without touching pre-existing locks | Two-user real fixture, hung-call/transaction-timeout/partial-effect fakes, durable journal and final lock proof |
| BD-10 | Relevant repository changes force a new distribution/rebase; unrelated commits do not; previewed repository update receives exact incoming add/delete only through the capability-proven structural confirmation | Relevant-object fingerprint fixtures, exact update digest, and two concurrent repository histories including add/delete |
| BD-11 | Main merge touches only approved delta/reference closure and preserves target support state | Pre/post original fingerprints and unsupported/upstream-supported real targets |
| BD-12 | Maximum configuration, UUID, reference, delta, diagnostics, and lock checks pass before commit | Immutable validation receipts produced while the exact owned lock set is held |
| BD-13 | Exact verified integration set, including add/delete entries without their own lock, is committed once with the frozen task-bound comment in one repository version and all acquired locks are released; ambiguous/partial outcomes never retry blindly | Real success, missing-lock, broken-reference, comment-policy, interruption, and disconnect fixtures |
| BD-14 | MCP/worker/process interruption resumes from durable state at every remote-effect barrier, a lost terminal response is recoverable from typed status handles and a digest-bound exact recovery plan, and a state-root override cannot hide an unresolved task or start attempt | Unit crash matrix, response-loss-after-observed tests for every handle-producing phase, recovery-plan approval tests, cross-root locator tests, fake worker death, real dangerous-phase restart scenarios |
| BD-15 | Successful archive is complete and redacted; disposable data is deleted only after commit/unlock/archive proof and marker identity matches project/original/repository journal identities | Archive schema tests, known-secret scan, owned-path/identity/quarantine tests and E2E |
| BD-16 | Abandoned tasks can be archived/cleaned only after original equality, no locks, no worker, and no unknown effect | State transition and unsafe-abandon rejection tests |
| BD-17 | Public surface is one server `unica`, 21 strict lifecycle tools, required general `unica.code.patch` and layer-aware `unica.support.edit`, and MCP-only `unica:branched-development` | Registry/schema tests, companion-tool contracts, skill transcript, source and generated-package smoke |
| BD-18 | A release claims only exact OS/platform/locale/original-IB/repository-transport capability rows proven by a disposable real fixture | Versioned capability manifest validated against hashed stored acceptance evidence |
| BD-19 | Credentials and known secret values never enter public result, journal, logs, archive, or MCP stdout | Typed-secret argv tests and byte scan of all retained fixture artifacts |
| BD-20 | Unknown metadata/support/platform behavior fails closed instead of using name-only or broad-lock approximations | Negative contract tests returning stable unsupported/capability errors |
| BD-21 | Full feature/data-migration tests finish in synchronized task IB before locks; the original gets no automatic `UpdateDBCfg`, DB restructuring, or destructive runtime test | Invocation-order assertion plus task validation receipts and original Designer transcript |
| BD-22 | Every tool obeys the exact generated request/result schema, per-variant mutability/preview policy, closed completed/stopped/rejected union, typed data/evidence, and operation replay rules | Committed generated JSON Schema/variant-policy snapshots plus application and packaged MCP contract tests, including evidence-preserving domain stops and data-free safety rejections |
| BD-23 | Every stable error code maps one trigger to the required phase/stop/recovery/cleanup behavior | Table-driven domain/application tests for the complete tool-contract error table |
| BD-24 | File and client/server originals plus file/server repository transports are enabled only by their own topology evidence | Separate disposable real-fixture rows; cross-topology negative preflight tests |
| BD-25 | Existing typed development, build, and test tools operate on the owned branch through `branchedTask` ID resolution, durable mutation receipts, and task-context cache events without exposing a task path in any result field | Application-port/schema tests plus packaged transcript covering code, metadata, build, and test calls, with recursive structured/free-text path-projection scans |

## Public Tool Matrix

This table is a completion summary; the exact schemas and data variants in the
tool contract are required, not optional implementation detail.

| Tool | Mutation boundary | Required guard or postcondition |
| --- | --- | --- |
| `unica.branched.start` | Durable journal and owned work root | Valid profile, task ID, operation ID, capability row, exclusive leases |
| `unica.branched.status` | None | Strictly read-only; first call after reconnect; returns current typed IDs/digests/previews needed for the next legal call without paths |
| `unica.branched.archive` | Durable archive | Verified success or verified safe abandonment; reason for abandonment |
| `unica.branched.cleanup` | Owned disposable root | Archived outcome, marker/nonce/containment/reparse checks, quarantine first |
| `unica.delivery.inspect` | None | Exact name/vendor/version identity, distribution/update permissions, warnings-as-errors, binding, repository/main/database equality, rules, support layers, stable digest |
| `unica.delivery.create` | Distribution artifact | Target-effect-free preview has no output ID/hash/time; apply uses a clean current repository head, no self-support, distribution command only, then returns proven artifact fields |
| `unica.delivery.verify` | Probe IB only | Prove artifact/support behavior; never trust extension; destroy probe only after durable observation |
| `unica.delivery.deploy` | Task File IB and workspace | Preview has no allocated IDs/post-effect fingerprints; apply uses verified distribution, owned destination, current=vendor proof and baseline fingerprints |
| `unica.merge.compare` | Evidence only | Typed allowed sides, UUID/property manifest plus platform report; reject name-only mapping |
| `unica.merge.prepare` | Fresh sandbox only | Immutable checkpoint/anchors and operation ID; resolved replay owns sandbox recreation and conflict-state exit |
| `unica.merge.conflicts` | None | All seven typed conflict kinds, three-side hashes, and per-kind allowed explicit resolutions |
| `unica.merge.resolve` | Decision journal | Typed payload, rationale, owned/fingerprinted manual data |
| `unica.merge.apply` | Named task/original target | Prepared/resolved session, intent-before-effect journal, fresh anchors, replay equality; task apply atomically publishes a validated full staged dump and proves IB/XML equality, while original also binds integration/lock sets and substantive repairs return to task |
| `unica.merge.verify` | Evidence/checkpoints | Local scope creates the first immutable checkpoint; equivalent/adapted synchronized-task scope creates the required post-update checkpoint; pre-lock main-sandbox and post-merge main-integration scopes produce immutable receipts |
| `unica.repository.status` | None | Binding, main/repository/database sync, journal/last-observed conflicts with completeness, nullable unproven owner fields; no global-live-lock or reconciliation claim |
| `unica.repository.update` | Original configuration | Target-effect-free exact preview/digest; no unowned local changes or unexpected task locks; structural confirmation only for approved incoming add/delete |
| `unica.repository.planLocks` | Evidence only | Exact integration add/modify/delete set plus independently acquired development-object/reference closure with reasons |
| `unica.repository.lock` | Repository locks | Approved exact plan, per-object journal, dedicated user, compensation proof; conflict returns object, nullable owner, redacted diagnostic and external action |
| `unica.repository.unlock` | Task-owned locks/config | No force; exact proven subset only |
| `unica.repository.commit` | Repository content/locks | Exact integration and acquired-lock sets, frozen template-rendered task comment, one call, no force/keepLocked, content/unlock proof |
| `unica.repository.recover` | Journaled recovery target | No new merge decision; ambiguity remains `recoveryRequired` |

All mutating tools require `taskId` and caller-stable `operationId`. Constant
safety behavior is not represented by user-selectable booleans. Every schema
rejects raw argv, executable paths, credentials, CFU, and raw/undocumented force
controls.

Contract tests additionally prove that distribution create cannot route through
ordinary `make`, merge apply rejects an unprepared/unresolved session,
repository lock rejects an unapproved plan, and cleanup rejects incomplete
terminal/archive proof.

Schema and skill transcripts distinguish all result variants. In particular,
twice-changed properties, unexpected delta, and a foreign lock return
`ok: false`, `resultKind: "stopped"`, their stable `stopCode`, typed evidence,
and resumable handles. Safety/precondition rejections return
`resultKind: "rejected"` and `TaskErrorData` only; they cannot masquerade as a
domain stop or reuse its evidence payload.

## State-Machine Evidence

Unit tests must enumerate every allowed and forbidden edge. Normal flow is:

```text
created -> preflightPassed -> baselineReady -> developing -> localVerified
-> synchronizationPrepared -> synchronized
-> integrationPlanned -> acquiringLocks -> locked -> mainMerged
-> mainValidated -> committing -> committedAndUnlocked
-> archivedSuccess -> cleanedSuccess
```

Only a session with conflicts branches
`synchronizationPrepared -> synchronizationConflicts -> synchronizationPrepared`.

Safe abandonment ends in `archivedAbandoned -> cleanedAbandoned`. The blocking
states `blockedByForeignLock`, `staleRelevantBaseline`, `unexpectedDelta`,
`lockPlanExpansionRequired`, `validationFailed`, `commitBlocked`, `recoveryRequired`, and
`committedUnverified` must have explicit exit tests. Neither
`recoveryRequired` nor `committedUnverified` can transition to archive or
cleanup. Tests use the exact exit guards/destinations in ADR-0012 rather than
merely asserting that some outgoing edge exists.

The lock-window failure tests prove both exact pre-effect exits: stale relevant
anchors retain the full owned lock set until verified unlock returns
`localVerified`, while an expanded lock requirement retains that set in
`lockPlanExpansionRequired` until verified unlock returns `synchronized` and
invalidates the main session, verification, plan, and lock evidence. Invalid
post-original verification must first enter `recoveryRequired`; only the exact
checkpoint-restore plus full-unlock recovery proof may enter
`validationFailed`.

An observed vendor-ancestry loss after request/digest validation must enter
`recoveryRequired` with a `taskConfiguration` checkpoint restore/recreate plan;
only a proven task fingerprint may return `localVerified`. Selecting a wrong or
stale input ID is instead a rejection and cannot fabricate this domain stop.

The foreign-lock exit test first proves compensation/no owned locks, then uses a
fresh clean inspection and applied refresh distribution to invalidate Dn-and-
later evidence and return `localVerified`. A later lock call makes exactly one
bounded observation; neither status nor the skill polls or treats external
coordination as proof of release.

Transition tests cover the guarded abandonment edge from every eligible phase
named by ADR-0012 and its rejection from lock/main-mutation/commit-ambiguity
phases. `unexpectedDelta` can reach `adapted` only through a recorded
verification/difference-digest decision with rationale followed by a fresh
matching verification.

Cancellation-path tests prove `locked` plus unchanged original can reach
`synchronized` only through complete `repository.unlock(reason="abandonment")`.
From `mainMerged`/`mainValidated`, the first archive preview must return the
exact no-effect abandonment recovery plan; only approved checkpoint restore,
before-state proof, and full unlock through `repository.recover` may return to
`synchronized` for a second, eligible archive preview. Commit ambiguity has no
such path.

Pending-plan tests prove status exposes exactly one current plan and every other
mutation is rejected. The no-effect abandonment preview can be cancelled only
through the digest-bound local recover-cancellation variant, which leaves the
main phase/anchors unchanged and invalidates the plan before normal
verification or commit resumes; effect-started recovery cannot be cancelled.

Each public tool that changes phase asserts its pre-state and writes its
postcondition before publishing success. `unica.merge.verify` owns the local,
synchronized-task, pre-lock main-sandbox, and post-merge main-integration
validation gates; it reruns the configured checks and stores receipts rather
than trusting prose supplied by a caller.

## Preflight And Lock-Window Evidence

Preflight records exact platform/configuration/repository identities, original
topology and repository transport, binding, compatibility mode, cleanup and
commit-comment policies, standard version, and optional Git branch/commit. Git
context is evidence only and never the merge base.

Before any repository lock, the synchronized task IB must have immutable
receipts for configured unit, integration, feature, data-migration, diagnostics,
syntax, and maximum safe local checks. While locks are held, only bounded
configuration/UUID/reference/delta/diagnostics/ownership checks run. The
invocation transcript must prove no `/UpdateDBCfg`, database restructuring, or
destructive runtime test targets the original infobase. A validation failure
uses the exact rollback/unlock exit and returns substantive repair to the
disposable task.

## Archive And Cleanup Schema

The versioned compact archive contains exactly the retained evidence classes:

- task manifest, external task/internal instance IDs, state transitions, and
  operation input/result digests;
- standard/platform/capability-row versions and original/repository identities;
- repository anchors and SHA-256 of D0, every refresh Dn, and ordinary result;
- pre/post canonical deltas and platform comparison reports;
- merge settings, typed conflict decisions, and manual change receipt digests;
- support-layer audit and proof that technical support did not enter original;
- lock plan plus acquisition/compensation/rollback journal;
- local, synchronized, main, and commit validation receipts;
- commit comment, exact object set, repository version/evidence, content proof,
  and released-lock proof;
- bounded redacted diagnostics and a manifest hash over every archive entry.

It excludes CF/XML/source bytes, database files, checkpoints, secrets or secret
hashes, raw logs, raw argv/stdout/stderr, and unbounded platform reports.

Successful or safely abandoned cleanup removes only the marker-owned task File
IB, XML workspace, probe/merge sandboxes, checkpoints, transient distribution
and ordinary CF artifacts, staged dumps, and non-archived logs. The compact
journal/archive remain. Every removed role appears in the preview and terminal
cleanup receipt; no unlisted path is traversed.

## Capability Fixture

The real fixture is opt-in and destructive only inside an explicit owned root:

```sh
UNICA_BRANCHED_ACCEPTANCE_CONFIG=/absolute/path/profile.yaml \
  cargo test -p unica-coder \
  --test platform_real_branched_development \
  -- --ignored --nocapture --test-threads=1
```

The profile names an exact Designer executable, empty test root, exact original
infobase kind, repository transport, disposable server provisioning when the
row is client/server, and test credentials through environment references. The
harness refuses root/home/Git paths and creates a unique marker-owned child, two
repository users, one disposable repository, and local File task/probe/sandbox
IBs. The original fixture is File or disposable client/server according to the
row; repository transport is independently file or server.

It records platform version, host OS/architecture, locale/encoding, topology,
contract digest, exact operation-class timeout map, implementation commit,
command result, redacted service
messages, artifact hashes, and every postcondition. It preserves the fixture on
an unknown effect and prints the recovery path; otherwise it quarantines and
deletes only its marked child. One topology's evidence never enables another.

One serialized run must cover:

1. ordinary CF versus distribution CF classification and D0 deployment;
2. precise existing-object support edit in unsupported and multi-vendor chains;
3. one-sided and twice-changed scalar and module updates;
4. reproducible `takeOurs`, `takeTheirs`, `combine`, and typed manual replay;
5. add/add UUID/name collision and top-level UUID preservation;
6. new form/attribute ownership and lock planning;
7. delete/modify and reference-cleanup planning without forced clearing;
8. foreign lock, bounded/hung-call timeout, partial acquisition, reverse
   compensation, and owner parsing;
9. same-user pre-existing lock behavior under the exclusive-user contract;
10. relevant and unrelated repository advancement, including an exact previewed
    add/delete update and capability-proven structural confirmation;
11. main merge support isolation and scoped fingerprints;
12. rollback/restore/unlock for modify, add, delete, interruption, and failure;
13. one integration-set commit with frozen task-bound comment,
    failure/ambiguity cases, and released-lock proof;
14. successful archive/cleanup, safe abandonment, and refusal on unsafe paths;
15. restart or worker death at every dangerous journal boundary, including task
    deployment and task supported-update apply before/after each effect marker.

The completion suite repeats applicable scenarios for File and client/server
originals and for every file/server repository transport claimed by the
capability manifest.

A capability manifest row is invalid if any required case is skipped or
verified only interactively without retained machine-readable evidence.

## Fake and Pure Test Layers

Required pure suites:

- transition table and terminal guards;
- operation replay and input-hash mismatch;
- atomic journal/schema migration and every write barrier;
- stable project/target/account identities, collision/relocation rules,
  cross-state-root original/account/task/session leases and reservations,
  non-overridable target locators, owner-only permissions, start-attempt replay,
  and unresolved-task exclusion;
- task ID normalization, pairwise work-root/state/coordination/original-workspace/
  File-IB/file-repository non-overlap, and destructive path/marker
  target-identity policy;
- typed secret/value redaction;
- canonical UUID/property delta and unsupported-kind rejection;
- metadata ownership and reference closure for modify/add/delete;
- layer-aware support parser/editor round trip;
- exact per-tool schema/envelope/data/evidence/error contract;
- read-only, proven-contained, and authoritative/unknown-effect timeout result,
  phase, durability, and recovery behavior;
- ordinary branched-task mutation phase rollback/descendant-evidence
  invalidation and session-scoped manual-resolution receipt expiry;
- worker socket/pipe token, PID/start-nonce binding, protocol, status, and
  policy-allowed cancellation;
- task-configuration recovery plans that restore/recreate from the exact
  checkpoint and never accept or blindly replay an unknown task mutation;
- merge settings, localized conflicts, decisions, and manual replay;
- resolved-replay refusal before every conflict is decided and before every
  resolution-workspace change is bound, including deterministic primary-code
  precedence when both defects exist;
- digest-bound supported-update session replacement after accidental unbound
  resolution edits, proving create-before-invalidate atomicity and expiry of the
  old workspace, decisions, and receipts;
- staged task-source publication, atomic replacement, cache events, and
  task-IB/XML fingerprint equality after authoritative replay;
- relevant anchors, lock planning, compensation, rollback, and commit
  reconciliation.

The fake Designer suite injects deterministic exits, output encodings,
twice-changed output, rejected merge settings, a hung read-only
`delivery.inspect` and capability-proven repository inspection, a hung contained
sandbox call, a hung lock process, partial lock success, compensation success,
compensation failure, a crash after a lock effect but before its
observed-journal write, stale anchors, extra lock requirements, secret-bearing
diagnostics, rollback success/failure, commit success, commit rejection before
effect, task-deploy/task-apply death at every journal barrier, partial commit,
ambiguous commit exit, and unlock failure after commit.
Each boundary is a separate case; read-only timeout must prove termination,
temporary-output disposal, unchanged phase, and absence of durable operation or
recovery state. One generic partial-effect test cannot satisfy several. The
suite verifies domain behavior but cannot create a capability manifest row.

OS-specific locator, process, filesystem, encoding, and reparse tests live under
`infrastructure/platform/**` or `tests/platform/**` so CI classification runs
the macOS/Linux/Windows matrix.

## Package and Skill Acceptance

The packaged skill is
`plugins/unica/skills/branched-development/SKILL.md` with public name
`unica:branched-development`. It is product-owned and routes only through the 21
public lifecycle tools, required `unica.code.patch`, the layer-aware
`unica.support.edit mode="layer"`, and other existing typed Unica tools. The
legacy broad support-edit mode is explicitly forbidden. The skill never
instructs the model to invoke Designer, `v8-runner`, shell, or a packaged
operation script directly.

For local development the transcript passes the original project `cwd` plus the
opaque `branchedTask` context selecting either `taskWorkspaceId` or an exact
session/digest-bound merge-resolution workspace. It never computes, prints, or
substitutes either disposable workspace path. Ordinary task mutations prove
phase rollback and descendant-evidence invalidation; manual/combine receipts
cannot escape their session.

The ordered E2E transcript must prove:

```text
status/recover -> start/preflight -> repository-clean D0 -> disposable work
-> local verification -> current D1 -> supported rebase/conflicts/delta proof
-> ordinary result -> main sandbox -> lock plan -> compensated acquisition
-> relevant-anchor check -> bounded original merge/validation
-> one commit/release -> verify -> archive -> cleanup
```

It must also prove that the skill stops on foreign locks, unknown effects,
unresolved conflicts, unsupported change kinds, and incomplete cleanup proof.
For an external dependency it emits terminal blocking guidance and does not ask
the user a generic “continue?”/confirmation question or poll automatically.
After external coordination it starts with `unica.branched.status`, then refreshes
repository/distribution/plan instead of reusing stale evidence.

Before the corresponding action, the transcript shows the user:

1. the exact support layer/object/rule that will become editable;
2. every foreign lock, nullable proven owner, redacted diagnostic, and requested
   external coordination;
3. every manual/combine conflict decision and rationale;
4. every expansion of the lock plan and its reasons;
5. the exact commit object set, validation digest, and comment;
6. cleanup eligibility, outcome, archive ID, and an opaque owned-target locator
   that does not expose the absolute disposable path.

The skill may report repository success only at `committedAndUnlocked`. It may
report full task completion only at `cleanedSuccess`, together with archive
location/hash and repository evidence. `cleanedAbandoned` is reported only as a
safely abandoned task and never as issue completion.

Package checks must include:

- all 21 tools in source and generated-package `tools/list`;
- required `unica.code.patch` and layer-aware support-edit schema/round-trip;
- an exact generated `supportsBranchedTask` registry snapshot covering the
  concrete project/configuration reads, code/support/metadata/form mutations,
  configuration build/load, diagnostics/syntax, and test operations used by the
  skill, with rejection tests for every other tagged operation;
- exactly one `.mcp.json` server named `unica`;
- `branched-development` in skill scenario and product-source provenance
  validation (`sourceKind`, owner repository, design path; no fabricated donor);
- no skill-local runtime scripts or raw Designer guidance;
- generated-package smoke on every release target;
- a byte scan showing that fixture secrets are absent from retained evidence;
- a recursive byte/value scan over every compatible general-tool response field
  proving that absolute task/work-root/state/coordination paths and path-bearing
  diagnostics never cross MCP.

## Completion Audit

Before closing #137, enumerate BD-01 through BD-25 and attach the authoritative
test/evidence path for each row. A green narrow suite cannot support a broader
row. Platform-version detection, documentation, fake output, or lack of a found
failure is not proof of a real repository postcondition.
