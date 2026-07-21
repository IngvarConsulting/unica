# Branched Development Executable Contract Plan

> **For agentic workers:** REQUIRED SUB-SKILL: execute every numbered task with
> `superpowers:subagent-driven-development`, use
> `superpowers:test-driven-development` for behavior, obtain a fresh independent
> review before marking a task complete, and use
> `superpowers:verification-before-completion` for every completion claim.

**Goal:** Build the complete, generated, closed Rust and JSON Schema contract
for all 21 ADR-0012 lifecycle tools before any of their handlers are registered.

**Authoritative baseline:** specification revision `1531d36`, plus the reviewed
Phase 0 kernel ending at the final Task 5A tracking commit. Code and tests remain
higher priority than prose. If an exact field/presence rule is not specified,
stop that affected type and repair the contract instead of inventing a field,
default, schema version, path, policy, or generic JSON escape hatch.

**Architecture:** A new `domain::branched_development::contracts` tree owns
strict Serde types, schema derivation, policy selection, and the descriptor
registry. The legacy `application::tool_contracts` key-name heuristic remains
unchanged for existing tools. The new registry is contract-only throughout this
plan: it does not add names to `application::tools()` or MCP `tools/list`.

## Corrections and resolved planning ambiguities

- The scope is 21 lifecycle tools, not 21 `unica.branched.*` tools. Exactly four
  names use that prefix; the other 17 use `unica.delivery.*`, `unica.merge.*`,
  and `unica.repository.*`. `BranchedLifecycleToolName` remains the exact
  canonical vocabulary and is not renamed.
- `TaskOperationToolName` is a separate closed union. During this plan its
  compatible-general-tool branch is empty because no current descriptor
  declares `supportsBranchedTask`; later Phase 4 additions are an explicit
  operation-record schema revision, never an inferred string escape hatch.
- The current MCP protocol advertises `2024-11-05` and publishes only
  `inputSchema`. This plan generates result schemas and policy metadata as
  registry artifacts, but does not add non-standard MCP fields or upgrade the
  protocol. Publication is a later protocol-specific change.
- Source fixtures live only under
  `tests/fixtures/branched_development/`. Phase 1 also creates a deterministic
  package-contract manifest containing their exact digests; it does not copy 21
  hand-maintained schemas into plugin metadata or misuse
  `third-party/tools.lock.json`. Once handlers are registered, package tests
  compare the built artifact's real `tools/list` schemas with these fixtures.
- The existing public Rust `OperationResult` cannot gain fields while preserving
  literal external struct construction. This plan preserves the serialized
  legacy envelope and all existing fields, introduces a closed internal
  `BranchedToolResult`, and records the deliberate Rust API expansion with
  compile tests. It does not claim impossible struct-literal source
  compatibility.
- The normative `OperationRecord` has no embedded `schemaVersion` or
  `schemaDigest`. Its expected current schema digest is the digest of the
  committed generated schema selected by the schema catalog. No field is added
  to the stored record. Versioned state framing/path selection belongs to Phase
  2 and must stay outside the record payload.

## Global implementation rules

- Use a pinned workspace `schemars` dependency and Serde derives/custom
  deserializers. Every object/variant is recursively closed with
  `additionalProperties: false`; no request, result, evidence, instruction,
  receipt, status, or storage field is `serde_json::Value`, an untyped map, or an
  untyped array.
- Schema-only bounds are insufficient. Bounded scalar/collection types must also
  reject invalid values during deserialization so MCP/storage behavior cannot
  depend on a cooperative client.
- Avoid `serde(flatten)` for request/presence unions. Use explicit variants so
  missing, extra, and cross-variant fields are rejected by both Serde and JSON
  Schema.
- Snapshot generation is deterministic, recursively key-sorted, newline-stable,
  and never silently updates fixtures. An ignored, explicitly opted-in
  regeneration test may write them; the ordinary test compares only.
- Request policy is selected from the deserialized closed variant. Unknown or
  malformed discriminators have no default policy.
- Keep every new registry/type `pub(crate)` unless a demonstrated application
  boundary requires a narrower deliberate public export.
- Do not register a handler, create a worker, touch state, or expose a prompt
  surface in this plan.

## Task 1: Schema foundation and bounded primitives

**Files:**
- Modify: `Cargo.toml`
- Modify: `crates/unica-coder/Cargo.toml`
- Modify: `Cargo.lock`
- Modify: `crates/unica-coder/src/domain/branched_development/identifiers.rs`
- Modify: `crates/unica-coder/src/domain/branched_development/vocabulary.rs`
- Create: `crates/unica-coder/src/domain/branched_development/contracts/mod.rs`
- Create: `crates/unica-coder/src/domain/branched_development/contracts/schema.rs`
- Create: `crates/unica-coder/src/domain/branched_development/contracts/scalars.rs`
- Modify: `crates/unica-coder/src/domain/branched_development/mod.rs`

- [ ] RED: assert exact schema and Serde rejection for `TaskId`, `OperationId`,
  `Sha256Digest`, `UnicaId`, `ProjectId`, `MetadataObjectId`,
  `ProfileArtifactRefId`, `CapabilityRowId`, `SupportLayerId`, bounded names,
  summaries, reasons, displays, property paths, diagnostics,
  `RepositoryVersion`, normalized UTC instants, positive generations, and
  bounded typed vectors.
- [ ] GREEN: add validated transparent newtypes and manual `JsonSchema`
  implementations where derive annotations cannot express runtime validation.
- [ ] Add a recursive schema audit helper that rejects an object schema without
  explicit closure and rejects untyped object/array leaves.
- [ ] Derive exact schemas for the existing task phase, execution policy,
  durable policy, and 21-name lifecycle vocabulary without changing wire
  spellings.
- [ ] Prove `readOnly` is absent from `DurableExecutionPolicy` schema.
- [ ] Run focused tests, format, clippy, full domain tests; commit and review.

## Task 2: Task and delivery request contracts

**Files:**
- Create: `crates/unica-coder/src/domain/branched_development/contracts/requests/mod.rs`
- Create: `crates/unica-coder/src/domain/branched_development/contracts/requests/task.rs`
- Create: `crates/unica-coder/src/domain/branched_development/contracts/requests/delivery.rs`

- [ ] RED: positive and missing/extra/cross-variant fixtures for start, status,
  archive, cleanup, inspect, create, verify, and deploy.
- [ ] Encode the normative closed `CommonTaskRequest { cwd, taskId }` and
  `CommonMutationRequest { cwd, taskId, operationId }` intersections explicitly
  in every branch. Start adds profile and immutable task summary; no request may
  substitute a task/disposable path for the original-project `cwd`.
- [ ] Encode preview/apply as closed unions: preview omits `dryRun` or supplies
  the literal `true` and has no approval; apply requires literal `dryRun: false`
  plus its exact approved digest. No generic required boolean or implicit apply
  is accepted.
- [ ] Encode archive's `reason` exactly for abandoned outcome and forbid it for
  success; restrict delivery artifact roles/kinds to request-legal subsets.
- [ ] Add per-type variant and policy methods; prove all eight tools have the
  contract policy and malformed values select no policy.
- [ ] Run focused/schema tests, format, clippy; commit and review.

## Task 3: Merge request contracts

**Files:**
- Create: `crates/unica-coder/src/domain/branched_development/contracts/requests/merge.rs`
- Modify: `crates/unica-coder/src/domain/branched_development/contracts/requests/mod.rs`

- [ ] RED: exact fixtures for compare, prepare, conflicts, resolve, apply, and
  verify, including every forbidden cross-scope ID and approval field.
- [ ] Model comparison sides and scope as closed variants; never accept a raw
  artifact/path selector.
- [ ] Intersect every branch with the exact common task/mutation record; do not
  omit `cwd`/`taskId` merely because each tool section lists only its specific
  fields.
- [ ] Model prepare's supported-update first/replacement/resolved-replay and
  main-integration branches separately. Only main integration selects
  `journaledEffect`; the other branches select `contained`.
- [ ] Model conflict and adapted-delta resolution separately. Manual/combine
  receipts are required exactly for the allowed conflict resolutions and absent
  elsewhere.
- [ ] Model task/original apply separately so plan/integration/lock/support-gate
  fields are required only for original apply.
- [ ] Model all four verify scopes and the adaptation pair presence rule.
- [ ] Run focused/schema tests, format, clippy; commit and review.

## Task 4: Repository request contracts and policy selection

**Files:**
- Create: `crates/unica-coder/src/domain/branched_development/contracts/requests/repository.rs`
- Modify: `crates/unica-coder/src/domain/branched_development/contracts/requests/mod.rs`

- [ ] RED: exact fixtures for status, update, planLocks, lock, unlock, commit,
  and recover, including all update/recovery stage branches.
- [ ] Preserve the special read-only `supportPrerequisiteArm/preview` branch:
  it retains `cwd`/`taskId` but has no operation ID, `dryRun`, or approval. Its
  apply retains the common mutation fields and approved arming digest but still
  has no `dryRun`.
- [ ] Encode routine/prerequisite/cancellation preview/apply unions and armed
  arming-ID/digest pair presence exactly. Do not introduce a generic approval.
- [ ] Encode digest approvals as typed records and validate the request-specific
  equality in constructors/domain validation rather than with caller flags.
- [ ] Encode recover apply and cancel-pending-plan as separate decisions; cancel
  is `localJournaled`, has no approval, and apply is `journaledEffect`.
- [ ] Exhaustively test every request variant-to-policy mapping and prove no
  unknown/default policy branch exists.
- [ ] Run focused/schema tests, format, clippy; commit and review.

## Task 5: Closed names, selectors, and descriptor registry skeleton

**Files:**
- Create: `crates/unica-coder/src/domain/branched_development/contracts/registry.rs`
- Create: `crates/unica-coder/src/domain/branched_development/contracts/selectors.rs`
- Modify: `crates/unica-coder/src/domain/branched_development/contracts/mod.rs`
- Modify: `crates/unica-coder/src/domain/branched_development/vocabulary.rs`

- [ ] RED: require descriptor cardinality/order/name equality with all 21 values
  of `BranchedLifecycleToolName::ALL` and fail if a second name list drifts.
- [ ] Define `TaskOperationToolName` as a closed lifecycle/general union without
  a string catch-all; the general branch is an explicit empty registry in this
  phase.
- [ ] Define exact `TaskOperationSelector` branches and request-variant literals
  used by next actions and digest errors.
- [ ] Add descriptors with name, request schema factory, variant metadata,
  policy selector, mutation/preview classification, and a deliberately absent
  handler binding.
- [ ] Prove the registry cannot be converted into current `ToolSpec` entries and
  none of its names appears in source `tools/list` yet.
- [ ] Run focused/schema/MCP regression tests; commit and review.

## Task 6: Common result envelope and rejected error contract

**Files:**
- Create: `crates/unica-coder/src/domain/branched_development/contracts/envelope.rs`
- Create: `crates/unica-coder/src/domain/branched_development/contracts/errors.rs`
- Create: `crates/unica-coder/src/domain/branched_development/contracts/actions.rs`

- [ ] RED: prove the completed/stopped/rejected presence matrix, stable-code
  cardinality, rejected-code subset, and all 11 literal code/context branches.
- [ ] Define non-empty/canonical error and next-action collections; a stopped
  primary error must equal `stopCode`, while rejected has no `stopCode`.
- [ ] Encode the 12 named redacted contexts and exact allowed-action grammar.
  Do not use a single code plus optional context bag.
- [ ] Make `completed` require `ok: true` and empty errors; stopped/rejected
  require `ok: false` and non-empty errors by construction.
- [ ] Keep `changes`, `artifacts`, and cache fields in every task-bound envelope;
  command/stdout/stderr/path/credential fields are unrepresentable.
- [ ] Add exhaustive cross-branch substitution negatives from contract lines
  1941-1990.
- [ ] Run focused/schema tests, format, clippy; commit and review.

## Task 7: Repository and artifact evidence types

**Files:**
- Create: `crates/unica-coder/src/domain/branched_development/contracts/repository.rs`
- Create: `crates/unica-coder/src/domain/branched_development/contracts/artifacts.rs`

- [ ] RED: repository cursor/history/partition, target identity/state/display,
  planned change, lock/update plan/proof, gate-history, post-merge guard, deferred
  advance/consumption, artifact/configuration identity and locator fixtures.
- [ ] Encode every tagged identity/state variant and optional-field matrix; root
  identity can never be absent and display text never substitutes for identity.
- [ ] Represent lineage with typed IDs/digests, not recursive object graphs.
- [ ] Add constructor/domain checks for canonical unique order and digest field
  formulas where the contract defines them.
- [ ] Prove CFU/invalid artifacts are classification evidence only and never
  accepted workflow input kinds.
- [ ] Run focused/schema tests, format, clippy; commit and review.

## Task 8: Support evidence, authorization, and instructions

**Files:**
- Create: `crates/unica-coder/src/domain/branched_development/contracts/support.rs`
- Create: `crates/unica-coder/src/domain/branched_development/contracts/instructions.rs`

- [ ] RED: all support transition/candidate/blocker/gap, manual target identity,
  baseline, root observation/proof, authorization/arming, version observation,
  recovery handoff/distribution and external instruction branches.
- [ ] Preserve exact manual-mode presence rules and explicit null-versus-absent
  fields. Do not fabricate object/layer IDs for global evidence gaps.
- [ ] Keep authorization terminal records distinct from active resume-handle
  projections.
- [ ] Encode instruction branches as closed data, never free-form prose or
  user-selectable effect booleans.
- [ ] Add negative mode/field splice tests and digest-lineage constructor checks.
- [ ] Run focused/schema tests, format, clippy; commit and review.

## Task 9: Change receipts and support terminalization

**Files:**
- Create: `crates/unica-coder/src/domain/branched_development/contracts/change_receipts.rs`
- Create: `crates/unica-coder/src/domain/branched_development/contracts/support_terminalization.rs`

- [ ] RED: all four `BranchedChangeReceipt` leaves, changed/no-change phase
  rules, target hashes/events/cache impact, recovery finalization plan/guard
  proof, working-IB closure plan/proof, and both mode stop-evidence types.
- [ ] Ensure no-change receipts cannot invalidate evidence or supersede a
  decision; changed receipts bind the exact invalidation/supersession closure.
- [ ] Keep conceptual workflow links as typed IDs/digests and ordinary owned
  fields; do not add `Box` or recursive generic JSON.
- [ ] Run focused/schema tests, format, clippy; commit and review.

## Task 10: Pre-arm cancellation and recovery core

**Files:**
- Create: `crates/unica-coder/src/domain/branched_development/contracts/prearm_recovery.rs`
- Create: `crates/unica-coder/src/domain/branched_development/contracts/recovery.rs`

- [ ] RED: pre-arm effect receipt/ref, selective update progress/evidence,
  observation, receipt plan, recheck policy/evidence/path/progress/audit/blocker,
  finalization plan, and all recovery subject/action/outcome/unknown branches.
- [ ] Preserve every effect barrier and receipt reference as a closed tagged
  union; `observed` must never become an operation state.
- [ ] Model reapproval and compensation evidence without overwriting immutable
  prior attempts.
- [ ] Prove unsupported target/effect/action combinations are unrepresentable.
- [ ] Run focused/schema tests, format, clippy; commit and review.

## Task 11: Status and storage schema types

**Files:**
- Create: `crates/unica-coder/src/domain/branched_development/contracts/status.rs`
- Create: `crates/unica-coder/src/domain/branched_development/contracts/storage.rs`
- Modify: `crates/unica-coder/src/domain/branched_development/operation.rs`
- Modify: `crates/unica-coder/src/domain/branched_development/operation_preflight.rs`

- [ ] RED: complete status projections, resume handles, recent terminal results,
  archive/cleanup status, operation lease, operation record, terminal envelope,
  and schema catalog snapshots.
- [ ] `ActiveOperationStatus.state` accepts only registered/intent-written/
  effect-unknown; terminal evidence is separate.
- [ ] `OperationRecord` uses only `DurableExecutionPolicy`, the exact four state
  variants, and the lease/terminal/recovery presence matrix. It has no embedded
  schema version/digest.
- [ ] Bind Task 5A opaque candidates to full typed deserialization and schema
  validation. Only this validated type may construct the replay view.
- [ ] Generate the expected operation-record schema digest from the committed
  normalized schema and prove exact stored byte hash remains the observed
  digest. Full filesystem/error mapping stays in Phase 2.
- [ ] Prove schema/status/storage reject `policy: readOnly` and no preflight
  failure can reach replay/status construction.
- [ ] Run focused/schema/doc compile-fail tests, format, clippy; commit and review.

## Task 12: Completed lifecycle and delivery result data

**Files:**
- Create: `crates/unica-coder/src/domain/branched_development/contracts/results/task.rs`
- Create: `crates/unica-coder/src/domain/branched_development/contracts/results/delivery.rs`
- Create: `crates/unica-coder/src/domain/branched_development/contracts/results/mod.rs`

- [ ] RED then implement all named completed schemas for start/status/archive/
  cleanup and inspect/create/verify/deploy.
- [ ] Encode preview versus apply outputs separately so previews cannot allocate
  post-effect IDs, hashes, fingerprints, receipts, or timestamps.
- [ ] Preserve not-created versus existing-task status as closed alternatives.
- [ ] Add path/process/credential recursive projection negatives.
- [ ] Run focused/schema tests, format, clippy; commit and review.

## Task 13: Completed merge and repository result data

**Files:**
- Create: `crates/unica-coder/src/domain/branched_development/contracts/results/merge.rs`
- Create: `crates/unica-coder/src/domain/branched_development/contracts/results/repository.rs`

- [ ] RED then implement the remaining named completed data schemas for compare,
  prepare, conflicts, resolve, apply, verify, repository status/update/planLocks/
  lock/unlock/commit/recover.
- [ ] Preserve target/scope/mode-specific presence rules and the intentional
  reuse of `MergeSessionData`/`MergeVerificationData` in stopped outcomes.
- [ ] Keep recovery apply and pending-plan cancellation outputs distinct.
- [ ] Run focused/schema tests, format, clippy; commit and review.

## Task 14: General, merge, and repository stopped data

**Files:**
- Create: `crates/unica-coder/src/domain/branched_development/contracts/stops/mod.rs`
- Create: `crates/unica-coder/src/domain/branched_development/contracts/stops/general.rs`
- Create: `crates/unica-coder/src/domain/branched_development/contracts/stops/merge.rs`
- Create: `crates/unica-coder/src/domain/branched_development/contracts/stops/repository.rs`

- [ ] RED then implement the exhaustive stop matrix outside support-specific
  reconciliation: artifact/platform/path, merge/session/verification/baseline,
  lock/update/commit/unlock, timeout and generic recovery-required records.
- [ ] Couple each stop code to exactly its allowed data type and producer set.
- [ ] Preserve evidence-bearing stopped data; never downgrade it to rejected
  `TaskErrorData` or generic diagnostics.
- [ ] Run exhaustive cross-producer/code/schema negatives, format, clippy;
  commit and review.

## Task 15: Support and recovery stopped data

**Files:**
- Create: `crates/unica-coder/src/domain/branched_development/contracts/stops/support.rs`
- Create: `crates/unica-coder/src/domain/branched_development/contracts/stops/recovery.rs`

- [ ] RED then implement all support preflight/cleanup/arm/prerequisite/
  cancellation/correction/conflict/reapproval/blocker/evidence pending records.
- [ ] Enforce target-mode and authorization-state presence rules, immutable prior
  attempt evidence, and exact required external instruction types.
- [ ] Cover all remaining rows in the normative 55-name stop matrix and assert
  there is no unmapped stable stop code.
- [ ] Run exhaustive schema negatives, format, clippy; commit and review.

## Task 16: Per-tool closed output unions and OperationResult projection

**Files:**
- Modify: `crates/unica-coder/src/domain/branched_development/contracts/envelope.rs`
- Modify: `crates/unica-coder/src/domain/branched_development/contracts/registry.rs`
- Modify: `crates/unica-coder/src/application/mod.rs`
- Test: `crates/unica-coder/src/application/mod.rs`

- [ ] RED: each of 21 descriptors must have an exact completed set, stopped
  code/data set, and common rejected set; mismatched stop/data/primary error is
  impossible in Rust and rejected by schema.
- [ ] Define internal `BranchedToolResult` variants and projection into the
  serialized application envelope without command/stdout/stderr leaks.
- [ ] Preserve every legacy serialized field/value for existing tools.
- [ ] Add compile/serialization tests documenting the deliberate Rust struct API
  expansion and wire compatibility; do not claim struct-literal compatibility.
- [ ] Keep application dispatch/registration absent.
- [ ] Run legacy application/MCP tests plus focused schema tests; commit/review.

## Task 17: Deterministic source snapshots and policy/storage manifests

**Files:**
- Create: `tests/fixtures/branched_development/tool_schemas/*.json` (21 files)
- Create: `tests/fixtures/branched_development/variant-policies.json`
- Create: `tests/fixtures/branched_development/storage-schemas.json`
- Create: `crates/unica-coder/src/domain/branched_development/contracts/snapshots.rs`
- Modify: `crates/unica-coder/src/domain/branched_development/contracts/mod.rs`

- [ ] RED: ordinary tests fail when any generated byte differs, a fixture is
  missing/extra, registry order drifts, or a schema contains an open/untyped
  object/array.
- [ ] Generate one deterministic combined request/result schema document per
  tool, the exhaustive variant-policy manifest, and storage/status schemas.
- [ ] Add an explicit ignored regeneration test guarded by an environment flag;
  ordinary tests never write.
- [ ] Assert exact count/name/error/policy sets, recursive closure, bounds,
  required fields, read-only durability exclusion, and raw path/process/secret
  field absence.
- [ ] Run the fixture check twice and prove a clean worktree on the second run.
- [ ] Commit generated artifacts and obtain independent review.

## Task 18: Package contract manifest and Phase 1 gate

**Files:**
- Create: `plugins/unica/references/branched-development/contracts-manifest.json`
- Modify: `scripts/ci/package-unica-plugin.py`
- Modify: `tests/ci/test_package_unica_plugin.py`
- Modify: `.superpowers/sdd/progress.md` (ignored durable ledger only)

- [ ] RED: package test fails unless the generated manifest names all 21 source
  fixtures plus policy/storage files with exact SHA-256 and rejects stale,
  missing, extra, duplicated, or unsafe paths.
- [ ] Generate/copy only the digest manifest into the tracked plugin tree; keep
  schemas single-sourced in test fixtures and keep `tools.lock.json` unchanged.
- [ ] Package tests verify the manifest survives tracked-source packaging and
  matches the checkout fixtures. They do not falsely assert handlerless
  `tools/list` registration.
- [ ] Run full contract, application, MCP, package, workspace, fmt, clippy,
  platform-boundary, and diff checks.
- [ ] Obtain final independent Phase 1 review. Record every RED/GREEN command,
  commit, and review in the progress ledger.

## Phase 1 completion evidence

- Exactly 21 registry descriptors match `BranchedLifecycleToolName::ALL` and
  none is registered as an MCP handler.
- Every descriptor has a strict generated request schema, strict generated
  closed result schema, and exhaustive variant policy metadata.
- All stable codes, rejected branches, completed data, stopped data, status,
  operation/lease/terminal/storage schemas are typed and snapshotted.
- There is no free-form JSON, raw command/process/path/credential field, durable
  `readOnly`, implicit policy/default discriminator, or open nested object.
- Source fixture bytes are deterministic and their package manifest digests
  match exactly.
- Existing generic tools preserve their source behavior and serialized MCP
  schemas.
- Full workspace tests, clippy with warnings denied, formatting, platform
  boundary, package tests, doc compile-fail boundaries, and `git diff --check`
  pass on the final Phase 1 commit.
