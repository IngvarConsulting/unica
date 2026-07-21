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
- Shared artifact vocabulary cannot first appear in Task 7: Task 2 already
  needs `AcceptedArtifactKind`, and Task 6 needs the full kind/role tuple.
  Task 2 therefore creates `contracts/artifacts.rs` with `ArtifactRole`,
  `ArtifactKind`, `AcceptedArtifactKind`, and the closed `ArtifactKindRole`;
  Task 7 extends that module with evidence records instead of redefining enums.
- Required-nullable Serde behavior cannot first appear in Task 7: Task 6's
  rejected contexts already need it. Task 6 creates the shared
  `RequiredNullable<T>` in `contracts/scalars.rs` and requires field-level
  `deserialize_with` on every physical required-nullable member. Tasks 7 and 8
  reuse that exact wrapper and deserializer.
- Exact action arrays require Draft 2020-12 positional schemas in Task 6. Task 6
  extends `contracts/schema.rs` with fail-closed `prefixItems` plus
  `items: false` support; later fixed tuples reuse it and never fall back to
  legacy array-valued `items`/`additionalItems`.
- `SupportMissingEvidenceKind` cannot first appear in Task 8 because Task 7's
  `DeferredRepositoryAdvance` already depends on it. Task 7 creates
  `contracts/support.rs` with that closed shared enum before compiling
  `repository.rs`; Task 8 modifies the module to add the remaining support
  records and never defines a second enum.
- Repository history has separate wire and domain types. Serde may create only
  `UnvalidatedRepositoryHistoryPartition`; a capability-backed
  `RepositoryHistoryOrderResolver` plus typed source resolution constructs the
  non-`Deserialize` `ValidatedRepositoryHistoryPartition`. No downstream
  control flow accepts the unvalidated DTO or infers order from opaque version
  strings.
- Task 7 creates the typed `EvidenceSourceRegistry` and capability-backed,
  version-indexed `EvidenceSourceIndex`, but registers only
  `routineClassification` and `nonConflictingConcurrent`. Its exact registry
  digest binds the evidence/digest-record schema digests and committed loader/
  mapper revision digests for every canonical entry; it does not invent a
  `CapabilityRowId` for code. Its authoritative proof reports exactly one
  available ref or explicit absence for every active kind. Selection uses the
  total active-kind precedence `supportPrerequisiteObservation >
  nonConflictingConcurrent > routineClassification`; every lower choice requires
  all active higher rows explicitly absent, and a wrong-class higher source is
  failure rather than fallback. Task 8 extends the same registry with
  `supportPrerequisiteObservation`, changes the registry digest, invalidates old
  proofs, owns the non-corrective mappings, and rejects `corrective` fail-closed.
  Task 9 enables both corrective branches through their distinct historical
  instruction/evidence validation. Although the wire enum already contains
  `taskCommit`, Task 7 rejects it during validated construction; Task 13 owns
  `RepositoryIntegrationEntry`, `CommitExactObject`,
  `CommittedRepositoryObject`, and the only crate-private constructor that
  validates a task commit inside its enclosing `CommitData`.
- Task 8 owns exactly the eight instruction records represented by
  `NextAction.externalInstruction`: acquire/release root locks, manual support,
  manual-IB/reserved-original closure, support conflict/evidence, and vendor
  decision. `SupportCorrectiveInstruction` and the closed
  `SupportRecoveryExternalAction` union first appear in Task 9, which modifies
  the same instruction module instead of creating a second vocabulary.
- Task 8 carries retention/manual-readability `CapabilityRowId` values only.
  Parsing and validating the retention-provider capability manifest remains
  roadmap Phase 3 work; the contract layer neither parses a row nor substitutes
  an arbitrary profile string.

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

- [x] RED: assert exact schema and Serde rejection for `TaskId`, `OperationId`,
  `Sha256Digest`, `UnicaId`, `ProjectId`, `MetadataObjectId`,
  `OriginalProjectCwd`, `LocalProfileName`, `ProfileArtifactRefId`,
  `CapabilityRowId`, `SupportLayerId`, bounded names, summaries, reasons,
  displays, property paths, diagnostics,
  `RepositoryVersion`, normalized UTC instants, positive generations, and
  bounded typed vectors.
- [x] Enforce `NormalizedUtcInstant` as validated uppercase RFC 3339 UTC with
  terminal `Z`, canonical optional nanosecond fraction, and no offset/lowercase/
  redundant-zero/leap-second spelling; serialization must preserve that one
  digest-stable representation.
- [x] GREEN: add validated transparent newtypes and manual `JsonSchema`
  implementations where derive annotations cannot express runtime validation.
- [x] Add a recursive schema audit helper that rejects an object schema without
  explicit closure and rejects untyped object/array leaves.
- [x] Derive exact schemas for the existing task phase, execution policy,
  durable policy, and 21-name lifecycle vocabulary without changing wire
  spellings.
- [x] Prove `readOnly` is absent from `DurableExecutionPolicy` schema.
- [x] Run focused tests, format, clippy, full domain tests; commit and review.

## Task 2: Task and delivery request contracts

**Files:**
- Create: `crates/unica-coder/src/domain/branched_development/contracts/artifacts.rs`
- Create: `crates/unica-coder/src/domain/branched_development/contracts/requests/mod.rs`
- Create: `crates/unica-coder/src/domain/branched_development/contracts/requests/task.rs`
- Create: `crates/unica-coder/src/domain/branched_development/contracts/requests/delivery.rs`

- [x] RED: positive and missing/extra/cross-variant fixtures for start, status,
  archive, cleanup, inspect, create, verify, and deploy.
- [x] Encode the normative closed `CommonTaskRequest { cwd, taskId }` and
  `CommonMutationRequest { cwd, taskId, operationId }` intersections explicitly
  in every branch. Start adds profile and immutable task summary; no request may
  substitute a task/disposable path for the original-project `cwd`.
- [x] Encode preview/apply as closed unions: preview omits `dryRun` or supplies
  the literal `true` and has no approval; apply requires literal `dryRun: false`
  plus its exact approved digest. No generic required boolean or implicit apply
  is accepted.
- [x] Encode archive's `reason` exactly for abandoned outcome and forbid it for
  success; restrict delivery artifact roles/kinds to request-legal subsets.
- [x] Define the shared artifact kind/role vocabulary once in `artifacts.rs`;
  request-only role literals remain narrower views and no later task may create
  duplicate artifact enums.
- [x] Add per-type variant and policy methods; prove all eight tools have the
  contract policy and malformed values select no policy.
- [x] Run focused/schema tests, format, clippy; commit and review.

## Task 3: Merge request contracts

**Files:**
- Create: `crates/unica-coder/src/domain/branched_development/contracts/requests/merge.rs`
- Modify: `crates/unica-coder/src/domain/branched_development/contracts/requests/mod.rs`

- [x] RED: exact fixtures for compare, prepare, conflicts, resolve, apply, and
  verify, including every forbidden cross-scope ID and approval field.
- [x] Model comparison sides and scope as closed variants; never accept a raw
  artifact/path selector.
- [x] `merge.conflicts` is the sole read-only branch: intersect it with
  `CommonTaskRequest` (`cwd`/`taskId`) and reject `operationId`. Intersect every
  other merge branch with `CommonMutationRequest`; no tool may omit `cwd` or
  `taskId` merely because its section lists only specific fields.
- [x] Model prepare's supported-update first/replacement/resolved-replay and
  main-integration branches separately. Only main integration selects
  `journaledEffect`; the other branches select `contained`.
- [x] Model conflict and adapted-delta resolution separately. Manual/combine
  receipts are required exactly for the allowed conflict resolutions and absent
  elsewhere.
- [x] Encode all four closed resolution request variants, but do not invent a
  static kind-to-resolution matrix. A resolved request is later checked against
  the persisted non-empty canonical `allowedResolutions`; schema generation may
  proceed, while handler registration stays gated on the fixture-backed
  classifier required by Phase 7.
- [x] Model task/original apply separately so plan/integration/lock/support-gate
  fields are required only for original apply.
- [x] Model all four verify scopes and the adaptation pair presence rule.
- [x] Run focused/schema tests, format, clippy; commit and review.

## Task 4: Repository request contracts and policy selection

**Files:**
- Create: `crates/unica-coder/src/domain/branched_development/contracts/requests/repository.rs`
- Modify: `crates/unica-coder/src/domain/branched_development/contracts/requests/mod.rs`

- [x] RED: exact fixtures for status, update, planLocks, lock, unlock, commit,
  and recover, including all update/recovery stage branches.
- [x] Preserve the special read-only `supportPrerequisiteArm/preview` branch:
  it retains `cwd`/`taskId` but has no operation ID, `dryRun`, or approval. Its
  apply retains the common mutation fields and approved arming digest but still
  has no `dryRun`.
- [x] Encode routine/prerequisite/cancellation preview/apply unions and armed
  arming-ID/digest pair presence exactly. Do not introduce a generic approval.
- [x] Encode digest approvals as typed records and validate the request-specific
  equality in constructors/domain validation rather than with caller flags.
- [x] Encode recover apply and cancel-pending-plan as separate decisions; cancel
  is `localJournaled`, has no approval, and apply is `journaledEffect`.
- [x] Exhaustively test every request variant-to-policy mapping and prove no
  unknown/default policy branch exists.
- [x] Run focused/schema tests, format, clippy; commit and review.

## Task 5: Closed names, selectors, and descriptor registry skeleton

**Files:**
- Create: `crates/unica-coder/src/domain/branched_development/contracts/registry.rs`
- Create: `crates/unica-coder/src/domain/branched_development/contracts/selectors.rs`
- Modify: `crates/unica-coder/src/domain/branched_development/contracts/mod.rs`
- Modify: `crates/unica-coder/src/domain/branched_development/vocabulary.rs`

- [x] RED: require descriptor cardinality/order/name equality with all 21 values
  of `BranchedLifecycleToolName::ALL` and fail if a second name list drifts.
- [x] Define `TaskOperationToolName` as a closed lifecycle/general union without
  a string catch-all; the general branch is an explicit empty registry in this
  phase.
- [x] Define exact `TaskOperationSelector` branches and request-variant literals
  used by next actions and digest errors.
- [x] Add descriptors with name, request schema factory, variant metadata,
  policy selector, mutation/preview classification, and a deliberately absent
  handler binding.
- [x] Expose no handler binding or registry-to-`ToolSpec` conversion API and
  prove none of its names appears in `application::tools()` or actual MCP
  `tools/list` yet. Do not claim absolute non-convertibility of a public-field
  application struct.
- [x] Run focused/schema/MCP regression tests; commit and review.

## Task 6: Common result envelope and rejected error contract

**Files:**
- Modify: `crates/unica-coder/src/domain/branched_development/contracts/mod.rs`
- Modify: `crates/unica-coder/src/domain/branched_development/contracts/schema.rs`
- Modify: `crates/unica-coder/src/domain/branched_development/contracts/scalars.rs`
- Modify: `crates/unica-coder/src/domain/branched_development/contracts/selectors.rs`
- Create: `crates/unica-coder/src/domain/branched_development/contracts/envelope.rs`
- Create: `crates/unica-coder/src/domain/branched_development/contracts/errors.rs`
- Create: `crates/unica-coder/src/domain/branched_development/contracts/actions.rs`

- [x] RED: prove the completed/stopped/rejected presence matrix, all 75
  `StableErrorCode` literals, all 30 `RejectedCode` literal leaves, and the 14
  grouped `TaskErrorData` code/context branches.
- [x] RED: reject an omitted required-nullable member while accepting explicit
  `null`/`T`, and reject every fixed tuple with a missing, extra, reordered, or
  cross-branch item. Snapshot exact Draft 2020-12 `prefixItems`, `items: false`,
  and equal tuple-length bounds; reject legacy/open-tail schemas in the recursive
  audit.
- [x] Add the shared `RequiredNullable<T>` in `scalars.rs`. Every physical field
  with required `T | null` wire semantics in Task 6 carries a field-level
  `#[serde(deserialize_with = "RequiredNullable::deserialize_required")]`, has
  no default/skip behavior, and distinguishes omission from explicit `null`.
  Tasks 7 and 8 must import this implementation rather than redefine it.
- [x] Extend `schema.rs` with fail-closed Draft 2020-12 fixed-tuple generation
  and recursive audit: require non-empty `prefixItems`, `items: false`, and
  `minItems == maxItems == prefixItems.length`; reject array-valued `items`,
  `additionalItems`, open tails, and length mismatches.
- [x] Define the closed generic envelope schema algebra over
  `C, W, A, K, E, D` as six distinct physical read-only/mutating ×
  completed/stopped/rejected structs. Keep their role-specific macro definitions
  and test byte-for-byte common property-set equality across all six; do not use
  `serde(flatten)` or invent a shared flattened `TaskResultFields` Rust struct.
  Later owner tasks bind
  `C`/`W`/`A` to exact named bounded item schemas and `K`/`E`/`D` to exact named
  recursively closed payload schemas before a concrete result schema exists.
  Do not add `Value`, untyped maps/arrays, free-form objects, or Task 6-invented
  placeholder payload records.
- [x] Define `TaskErrorEntry`; completed has exactly zero errors, while stopped
  and rejected each have exactly one error whose code respectively equals
  `stopCode` or `TaskErrorData.code`. Secondary error entries are illegal.
- [x] Add crate-private typed selector constructors in `selectors.rs`; action
  construction accepts concrete selector/variant enums and creates
  `TaskOperationSelector` directly. No raw strings, `serde_json::Value`, parsing,
  or JSON serialize/deserialize round trip is an internal construction path.
  Derive canonical ordinals from the global selector table.
- [x] Encode the 15 named redacted contexts, 53 lifecycle operation selectors,
  eight external-instruction literals, and their exact canonical/duplicate-free
  allowed-action grammar, including the singleton adaptation-refresh selector
  and exact `taskNotFound` actions `[branched.start, branched.status]`. Name that
  grammar `startAndStatus`; array order is canonical set order, not an execution
  instruction. Do not use a single code plus optional context bag. Task 6 closes
  the 30 rejected leaves only; it does not assign a common producer set.
- [x] Give `stateCorrupt` its exact trusted five-way state reference and closed
  observation union: read bytes use `exactBytes { observedDigest }`, while an
  absent or ACL-inaccessible expected object uses `unavailable { reason:
  missing | permissionDenied }`. Forbid sentinel/metadata digests and loose
  optional fields; retain existing bytes untouched, while missing state has no
  fabricated object to retain.
- [x] Make `completed` require `ok: true` and empty errors; stopped/rejected
  require `ok: false` and the exact singleton error by construction.
- [x] Require top-level `operationId` in every completed/stopped/rejected result
  for a `DurableExecutionPolicy` and require it to equal the request value;
  forbid it for every `readOnly` result. A typed `data` reference to a
  pre-existing operation does not create a top-level read-only exception.
- [x] Keep `changes`, `warnings`, `artifacts`, `cache`, `evidence`, and `data` in
  every task-bound envelope with their exact generic bindings;
  command/stdout/stderr/path/credential fields are unrepresentable.
- [x] Add exhaustive cross-branch substitution negatives from the normative
  `TaskErrorData` and rejected-code presence/action table. Runtime validation
  rejects every invalid relational substitution whose compared values are in
  the wire context; schema validation rejects all structurally expressible
  substitutions. Exact-projection lists whose authoritative component
  preimages are intentionally not on the wire remain producer/replay-validator
  invariants owned by Task 16 and Phase 2. Enumerate and freeze the narrow
  Draft 2020-12 schema supersets for cross-field equality/inequality/membership
  rather than inventing a hidden discriminator. Do not rely on stale source
  line numbers.
- [x] Run focused/schema tests, format, clippy; commit and review.

## Task 7: Repository and artifact evidence types

**Files:**
- Modify: `crates/unica-coder/src/domain/branched_development/canonical_json.rs`
- Modify: `crates/unica-coder/src/domain/branched_development/contracts/mod.rs`
- Modify: `crates/unica-coder/src/domain/branched_development/contracts/scalars.rs`
- Create: `crates/unica-coder/src/domain/branched_development/contracts/repository.rs`
- Create: `crates/unica-coder/src/domain/branched_development/contracts/support.rs`
- Modify: `crates/unica-coder/src/domain/branched_development/contracts/artifacts.rs`

- [x] RED: repository cursor/history/partition, target identity/state/display,
  planned change, lock/update plan/proof, gate-history, post-merge guard, deferred
  advance/consumption, artifact/configuration identity and locator fixtures;
  include exact RFC 8785 serializer vectors and invalid I-JSON inputs. Keep the
  raw serializer assertion `1E30 -> 1e+30` separate from the typed contract
  helper assertion that rejects that integer-valued number above `2^53 - 1`;
  do not claim the raw RFC vector is admissible contract data. Cover Task 7-owned
  `routineClassification`/`nonConflictingConcurrent` semantic-delta mappings,
  each named evidence digest record, the exact `CanonicalEmptyDeltaDigest`
  literal, equal-endpoint/empty-entry partition fixtures plus both invalid
  endpoint/emptiness cross-pairs, and schema/Serde negatives that distinguish an
  omitted required-nullable key from an explicit `null`. Prove the wire schema
  can name later support/task-commit branches while Task 7 validation rejects
  both. Cover source-index explicit absence, missing registry rows, multiple
  refs, wrong registry/version/kind/ref/digest, and caller-selected-source
  negatives. Assert `availability` is byte-for-byte aligned with registry-entry
  evidence-kind order; any reorder is rejected and changes the proof digest.
  Cover registry-entry missing/extra/duplicate/order and every
  evidence-schema/digest-record-schema/loader-revision/mapper-revision digest
  substitution.
- [x] Freeze the Draft 2020-12 structural-superset boundary. Schema tests reject
  open/unknown fields, wrong tags, missing required or required-nullable keys,
  wrong nullability, wrong physical leaves, scalar bounds, and every exact tuple
  error. Serde/private constructors/resolvers additionally reject semantic-key
  order/uniqueness, endpoint/emptiness and adapter-order disagreement, digest
  recomputation failures, registry/index row misalignment, source-precedence and
  nested-ref disagreement, plan/proof byte inequality, and non-reverse release.
  Enumerate these unavoidable relational schema supersets in tests instead of
  claiming JSON Schema compares sibling values or hashes preimages.
- [x] Define `SupportMissingEvidenceKind` first in `support.rs`, re-export it
  through `contracts/mod.rs`, and reuse it from repository and later support
  records. Do not copy the literal vocabulary into either consumer.
- [x] Encode every tagged identity/state variant and optional-field matrix; root
  identity can never be absent and display text never substitutes for identity.
  Keep `RepositoryTargetKind` distinct from merge/apply `TargetKind`, enforce
  the exact target/reason order, and enforce the 0-or-1/null and scalar bounds
  of owner, actor, vendor, and version fields.
- [x] Reuse Task 6's `RequiredNullable<T>` and shared required deserializer for
  every Task 7 required-nullable member. Each physical field carries the
  field-level `deserialize_with`, the containing closed record requires the key,
  and omission remains distinct from explicit `null`; do not redeclare the
  wrapper or add `Option<T>` defaults/skip attributes.
- [x] Represent lineage with typed IDs/digests, not recursive object graphs.
- [x] Define Task 7's reusable `CanonicalEmptyDeltaDigest` value type/constant
  as exactly `sha256(canonical([]))`; its schema and deserializer accept only
  that one lowercase digest. Task 8 imports it for positive support-version
  observations rather than accepting an arbitrary `Sha256Digest` or recomputing
  a second constant.
- [x] Type `DeferredRepositoryAdvanceConsumptionReceipt.resultingPhase` as the
  closed `TaskPhase` and include it in the named receipt digest record. Task 7
  does not invent a narrower standalone phase set; the later enclosing
  `RepositoryUpdateData` constructor must bind it byte-for-byte to its validated
  result phase and enforce the selected update mode's phase rules.
- [x] Treat `RepositoryVersion` as opaque. Ordinary Serde produces only the
  closed `UnvalidatedRepositoryHistoryPartition`, which has no domain methods.
  A capability-backed `RepositoryHistoryOrderResolver` with internal typed
  order evidence accepts an empty partition if and only if its endpoints are
  byte-identical and `entries` is empty. Only for a non-empty partition does it
  prove immediate succession, complete coverage, endpoints, canonical adapter
  order, and uniqueness before constructing the non-`Deserialize`
  `ValidatedRepositoryHistoryPartition`; numeric/lexical version inference is
  forbidden. Static assertions/compile-fail tests prove the validated type has
  no `Deserialize` implementation and no domain API accepts the unvalidated DTO.
- [x] Add validated collection newtypes/private constructors for target states,
  planned changes, lock targets/reasons, and acquire/release sequences. Their
  Serde paths enforce each applicable self-contained invariant: canonical order
  and uniqueness for every identity collection; non-empty canonical reason
  lists and completed lock/acquire/release sequences; and exact reverse release
  through the enclosing constructor. Target-state and planned-change lists may
  be empty, including the declared cancellation case. Partition order/coverage
  instead requires the resolver above. A bare length-bounded `BoundedVec` must
  not bypass either path.
- [x] Extend the existing production JCS implementation in the parent
  `canonical_json.rs` with one typed, `pub(super)` fail-closed digest helper for
  sibling contract modules; do not create a second contract-local JCS path. It
  accepts only types with an explicit internal `ContractDigestRecord` marker
  implementation; arbitrary `Serialize`/`serde_json::Value` is not a production
  call path. Private record fields and validated constructors provide the typed
  schema boundary. Before hashing, the helper must both exercise the canonical
  serializer on the original value (so NaN/infinity cannot become `null`) and
  perform duplicate-preserving serialization followed by the shared strict
  parser (so duplicate names and unsafe integers cannot be coalesced). It hashes
  only the one canonical byte sequence after both views agree. It otherwise
  accepts only schema-valid I-JSON, emits RFC 8785 UTF-8 bytes, rejects duplicate
  names/lone surrogates/non-finite or out-of-range numbers and every
  canonicalization failure, and has no `serde_json::to_string`, debug-format,
  local `sha2`, or fallback hashing path. Task 8 reuses this helper.
- [x] Add constructor/domain checks for the exact semantic-delta input/null
  mapping and the named canonical digest records for gate-history, post-merge
  guard, selective update proof, and original-clean refresh proof. Every
  non-task-commit entry requires the closed content-addressed
  `RepositoryHistorySourceEvidenceRef` plus an internal
  `EvidenceSourceIndexProof` for the same version and active registry digest.
  Implement the named closed registry entry and `{ entries }` digest record with
  exact evidence/digest-record schema digests plus committed typed loader and
  classification-mapper revision digests. Schema-digest preimages are the exact
  standalone Draft 2020-12 documents, including `$schema`, title, and reachable
  `$defs`, with no postprocessing. Loader revision preimages are the closed
  `EvidenceLoaderRevisionDigestRecord` with the exact six validation checks;
  mapper revision preimages are the closed physical
  `EvidenceClassificationMapperRevisionDigestRecord` leaves with exact ordered
  source-to-partition rows and all six semantic digest-slot projections. Commit
  every lowercase digest constant and recompute it from its named preimage in
  tests. Entries are non-empty, unique, and in
  evidence-kind declaration order. Do not use arbitrary version strings,
  function/debug identities, or a synthetic `CapabilityRowId`. Task 7 registers
  only routine-classification and non-conflicting-concurrent loaders; each
  canonical proof has one available ref or explicit absence per registered kind.
  Select non-conflicting-concurrent over routine; routine is legal only with an
  explicit absent higher row, and a wrong-class available higher source fails.
  Deterministic lookup resolves and rehashes the proof-selected typed record, and
  the concurrent inline copy must match it. Missing/multiple/unregistered
  sources, incomplete/stale index proofs, ref substitution, semantic mappings,
  order evidence, or digests produce no validated partition. Late audit performs
  the same lookup and index validation again.
- [x] Keep `taskCommit` in the wire classification enum but make Task 7's generic
  validated constructor reject it. Do not introduce a placeholder committed-
  object type, trust its opaque semantic digest, or expose a public bypass; Task
  13 owns the enclosing validation.
- [x] Keep `ArtifactKind` as classification output and prove CFU/invalid
  artifacts are never accepted by a selectable workflow input kind/role.
- [x] Run focused/schema tests, format, clippy; commit and review.

## Task 8: Support evidence, authorization, and instructions

**Files:**
- Modify: `crates/unica-coder/src/domain/branched_development/contracts/mod.rs`
- Modify: `crates/unica-coder/src/domain/branched_development/contracts/repository.rs`
- Modify: `crates/unica-coder/src/domain/branched_development/contracts/support.rs`
- Create: `crates/unica-coder/src/domain/branched_development/contracts/instructions.rs`

- [ ] RED: all support transition/candidate/blocker/gap, manual target identity,
  baseline, root observation/proof, authorization/arming, version observation,
  recovery handoff/distribution and the eight `NextAction` external-instruction
  branches. Cover every exact digest record, discriminator, collection order,
  duplicate, explicit-null, cross-mode, and omitted/extra-field negative.
- [ ] Register `SupportPrerequisiteVersionObservation` as the exact typed
  `supportPrerequisiteObservation` content-addressed history source and extend
  the Task 7 registry/index proof with that exact third kind and its evidence-
  schema, digest-record-schema, loader-revision, and mapper-revision digests.
  Implement its
  routine/authorized/external-support/pre-arm/invalid classification and
  semantic-null mappings. Recompute the full registry digest so every old two-
  kind proof fails. Enforce total precedence: select support whenever available;
  select non-conflicting-concurrent only with support explicitly absent; select
  routine only with both support and non-conflicting rows explicitly absent.
  Reject any substituted registry-entry digest, missing/multiple refs, wrong-
  kind/digest/version/ref, wrong-class available higher sources, and every
  fallback that skips precedence. Keep the wire
  `corrective` leaf structurally representable but reject it during validated
  construction until Task 9 can bind it to the exact historical corrective
  instruction. Until this Task 8 extension succeeds, every raw support-backed
  partition remains unvalidated and unusable.
- [ ] Preserve exact manual-mode presence rules and explicit null-versus-absent
  fields by reusing Task 6's `RequiredNullable<T>` and its field-level required
  deserializer on every physical required-nullable member. Do not fabricate
  object/layer IDs for global evidence gaps. Treat `SupportRootLockProof`
  terminalization presence as an outer authorization/result invariant because
  the nested proof has no target mode; never infer one.
- [ ] Give all four `SupportTransition` leaves the required `transitionKind`
  discriminator. Implement the closed `SupportCandidateReason` and
  `VendorSupportDecision` vocabularies and validated semantic collection types
  for candidates, blockers, gaps, transitions, conflicts, mismatch/missing-kind
  projections, and allowed decisions. Comparators use typed identities and
  capability-proven history order, never displays or lexical/numeric ordering of
  opaque `RepositoryVersion`.
- [ ] Implement the acyclic named digest-input records for candidate set,
  replaceable-history-independent support gate, immutable support action,
  manual-IB identities/baselines, reserved-original/root observations/proofs,
  arming/stale/inventory/lease evidence, and instruction digests. Reuse the
  existing Task 7 `pub(super)` canonical helper; no local JCS/SHA path is legal.
- [ ] Keep authorization terminal records distinct from active resume-handle
  projections. `supportActionDigest` excludes its own field plus mutable
  arming/state/freeze members; `supportGateDigest` excludes the replaceable
  history evidence/cursor and action projection.
- [ ] Encode exactly `AcquireSupportRootInstruction`,
  `ReleaseRepositoryLocksInstruction`, `ManualSupportInstruction`,
  `CleanManualWorkingInfobaseInstruction`,
  `CloseReservedOriginalDesignerInstruction`, `SupportConflictInstruction`,
  `SupportEvidenceInstruction`, and `VendorSupportDecisionInstruction` as closed
  data. Add and verify `lockInstructionDigest` and
  `supportEvidenceInstructionDigest`; no free-form prose or user-selectable
  effect boolean is legal.
- [ ] Keep retention-provider and manual-readability fields as typed
  `CapabilityRowId` references only. Do not implement the Phase 3 manifest-row
  parser or duplicate its case vocabulary in Task 8.
- [ ] Add negative mode/field splice tests and digest-lineage constructor checks.
- [ ] Run focused/schema tests, format, clippy; commit and review.

## Task 9: Change receipts and support terminalization

**Files:**
- Modify: `crates/unica-coder/src/domain/branched_development/contracts/mod.rs`
- Modify: `crates/unica-coder/src/domain/branched_development/contracts/instructions.rs`
- Modify: `crates/unica-coder/src/domain/branched_development/contracts/repository.rs`
- Modify: `crates/unica-coder/src/domain/branched_development/contracts/support.rs`
- Create: `crates/unica-coder/src/domain/branched_development/contracts/change_receipts.rs`
- Create: `crates/unica-coder/src/domain/branched_development/contracts/support_terminalization.rs`

- [ ] RED: all four `BranchedChangeReceipt` leaves, changed/no-change phase
  rules, target hashes/events/cache impact, recovery finalization plan/guard
  proof, working-IB closure plan/proof, both mode stop-evidence types,
  `SupportCorrectiveInstruction`, `SupportRecoveryExternalAction`, and both
  instruction-bound corrective observation/source-resolution leaves.
- [ ] Extend Task 8's instruction module with the corrective instruction and the
  closed recovery-external-action union; reuse the existing eight instruction
  records and their digests without redefining any `NextAction` vocabulary.
- [ ] Extend the Task 8 support-observation source validator to accept
  `corrective` only by its `correctionKind`. `actionCorrection` resolves and
  rehashes the historical `SupportCorrectiveInstruction`, then proves exact
  instruction digest, repository actor, target-mode/working-IB presence, and
  root/content delta equality. `externalConflictCorrection` instead resolves and
  rehashes Task 8's historical `SupportConflictInstruction`, then proves its
  instruction digest, `conflictResolutionId`, `finalBaselineDigest ==
  requiredFinalBaselineDigest`, and the exact `ExternalSupportOwnershipEvidence`
  binding the same actor/version/root/content delta. Both leaves require the
  authoritative source-index version/ref, partition class `corrective`,
  recomputed classification and semantic-delta digests. Add missing/multiple,
  wrong-kind/version/ref/digest, cross-action/conflict, and cross-leaf negatives;
  structural observation validity alone must never authorize either branch.
- [ ] Enabling those corrective mappings changes the support-observation
  registry entry's `classificationMapperRevisionDigest`, recomputes
  `registryDigest`, and rejects every Task 8 `EvidenceSourceIndexProof`. Keep the
  evidence-schema, digest-record-schema, and loader revision digests unchanged
  unless their actual schema/loader changes. Test stale Task 8 proof rejection,
  mapper-digest substitution, and the recomputed Task 9 proof/order.
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
  archive/cleanup status, operation lease, and the generic operation-record
  presence matrix. RED also substitutes every lease digest/input and the
  terminal-envelope digest; JSON Schema shape success must not bypass semantic
  hashing. Instantiate storage tests only with one private recursively
  closed `TestTerminalEnvelope`; assert that no production terminal alias,
  terminal catalog, operation-record snapshot, or expected digest exists yet.
- [ ] `ActiveOperationStatus.state` accepts only registered/intent-written/
  effect-unknown; terminal evidence is separate. Status projects the exact typed
  `operation: TaskOperationSelector`, not a variant-losing `toolName` field.
- [ ] Define only crate-private `OperationRecord<TerminalEnvelope>`. Its generic
  structure uses `DurableExecutionPolicy`, the exact four state variants, and
  the lease/terminal/recovery presence matrix; it has no embedded schema
  version/digest. Persist exactly one typed `operation: TaskOperationSelector`
  producer discriminator and forbid sibling `toolName`/`requestVariant` fields;
  the one-way `canonicalInputDigest` cannot recover a physical variant.
  Production code cannot instantiate or alias it in this task.
- [ ] Add the closed `OperationScope` union: pre-task, original-workspace-scoped `startAttempt`
  contains the canonical original-workspace identity digest and `taskId`, while
  normal `task` scope contains `projectId`, `taskId`, and `instanceId`. The
  generic loader compares the complete scope with the start-attempt storage key
  or authoritative parent task record/locator before constructing any status or
  replay view; copied/cross-scope records fail. Do not persist a path or hash the
  caller's unnormalized `cwd` spelling.
- [ ] Validate `heartbeatDigest` and `leaseDigest` from their exact canonical
  records, and validate
  `terminalEnvelopeDigest == sha256(canonical(terminalEnvelope))` generically.
  These value checks are mandatory after shape validation and before any replay
  view; no caller-supplied digest is trusted merely because its scalar schema is
  valid.
- [ ] Keep Task 5A opaque candidates unbound to a production record. A test-only
  typed loader may validate `OperationRecord<TestTerminalEnvelope>` and construct
  a test replay view; the production opaque-loader binding waits for Task 16.
- [ ] Prove the generic schema/status/storage foundation rejects
  `policy: readOnly` and no poison/preflight failure can reach replay/status
  construction. Read-only selector-variant and terminal-producer binding waits
  for the real result union in Task 16.
- [ ] Do not generate or commit the final operation-record schema, expected
  digest, schema catalog, or storage snapshots. Task 17 first does so after
  Tasks 12-16 close and bind every production terminal variant.
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
- Modify: `crates/unica-coder/src/domain/branched_development/contracts/repository.rs`
- Create: `crates/unica-coder/src/domain/branched_development/contracts/results/merge.rs`
- Create: `crates/unica-coder/src/domain/branched_development/contracts/results/repository.rs`

- [ ] RED then implement the remaining named completed data schemas for compare,
  prepare, conflicts, resolve, apply, verify, repository status/update/planLocks/
  lock/unlock/commit/recover. For commit, cover all three
  `CommittedRepositoryObject` leaves, forbidden cross-leaf fields, empty/
  noncanonical/duplicate targets, plan/preview/result projection mismatch,
  wrong result version/post-state, digest substitution, and task-commit
  version/digest mismatch. Also cover every root/dev add/modify/delete
  `RepositoryIntegrationEntry`/`CommitExactObject` leaf, cross-target/action
  splice, empty/reordered/duplicate lists, empty/duplicate/reordered reasons,
  noncanonical lock targets, and any presentation/reason/lock field leaked into
  the exact-object schema/hash.
- [ ] Define named closed `$defs.RepositoryIntegrationEntry` leaves with the
  exact nested `RepositoryTargetIdentity`, presentation-only `objectDisplay`,
  root-modify or dev add/modify/delete action, non-empty canonical typed
  integration reasons, and canonical unique `requiredLockTargets`. Preserve
  add/delete entries even when their target itself is not separately lockable.
  For delete, include the self target iff capability evidence proves it still
  exists and is separately lockable at acquisition; parent/subordinate/changed-
  referrer targets remain mandatory. Test both capability branches and prove
  absence of a self-lock never removes the delete integration/commit entry.
- [ ] Define named closed `$defs.CommitExactObject` as only the four canonical
  target/action projections, with no display/reason/lock field. Require
  `CommitPreviewData.exactObjects` to be the non-empty canonical unique one-to-one
  projection of approved `LockPlanData.integrationEntries`, and compute
  `exactObjectsDigest` only from that projection.
- [ ] Define named closed `$defs.CommittedRepositoryObject` as exactly root-
  modify, development-object add/modify-present, and development-object delete-
  absent leaves. Require a non-empty canonical unique target list and the exact
  equality chain `project(integrationEntries) == exactObjects ==
  project(committedObjects)`. Present-leaf versions and absent-leaf establishment
  versions equal `CommitData.repositoryVersion`; fingerprints/absence equal the
  verified post-state. Recompute `committedObjectsDigest` from the named closed
  `{ integrationSetDigest, committedObjects }` record, with the digest equal to
  the approved LockPlan/CommitPreview value. Keep `exactObjectsDigest` as the
  separate identity/action projection check; it must not replace or weaken the
  full integration-set lineage bound into the task-commit digest.
- [ ] Implement the only crate-private task-commit partition constructor. It
  validates the exact singleton task version and its semantic digest equality to
  `committedObjectsDigest`, the whole capability-proven history range, and every
  other entry through the authoritative source index/ref proof. No generic
  constructor or raw DTO can accept that branch.
- [ ] Preserve target/scope/mode-specific presence rules and the intentional
  reuse of `MergeSessionData`/`MergeVerificationData` in stopped outcomes.
- [ ] Define the exact canonical `CommitCommentPolicyDigestRecord` and
  `IntegrationSetLineageDigestRecord` preimage types used by rejected mismatch
  proofs; do not leave either as an opaque digest producer or reconstruct a
  historical observed policy record from current mutable profile state.
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
- [ ] In the outer `SupportPreflightStopData` constructor, require the sibling
  `preflight.supportActionId`/`supportActionDigest` projection to equal the
  `SupportActionAuthorizationData` record byte-for-byte. Do not look for a
  nonexistent authorization nested inside `SupportPreflightData`. Validate
  stopped `SupportRootLockProof` terminalization-digest presence against the
  same outer authorization target mode.
- [ ] Cover all remaining support/recovery rows. Combined Tasks 14 and 15 must
  exhaust the current normative matrix's 45 stable stop-code names across 58
  producer rows and assert there is no unmapped or duplicate code/row.
- [ ] Run exhaustive schema negatives, format, clippy; commit and review.

## Task 16: Per-tool closed output unions and OperationResult projection

**Files:**
- Modify: `crates/unica-coder/src/domain/branched_development/contracts/envelope.rs`
- Modify: `crates/unica-coder/src/domain/branched_development/contracts/registry.rs`
- Modify: `crates/unica-coder/src/domain/branched_development/contracts/storage.rs`
- Modify: `crates/unica-coder/src/domain/branched_development/operation.rs`
- Modify: `crates/unica-coder/src/domain/branched_development/operation_preflight.rs`
- Modify: `crates/unica-coder/src/application/mod.rs`
- Test: `crates/unica-coder/src/application/mod.rs`

- [ ] RED first: derive from the stated per-variant precondition semantics and
  freeze an explicit normative 53-by-30 `RejectedProducerMatrix`: one row for
  every physical lifecycle selector variant, one column for every Task 6
  `RejectedCode` leaf, and an exact legal/illegal value for all 1590 cells.
  Tests enumerate every cell and prove missing-task status is completed
  `notCreated`, while start, status, and other read-only variants have their own
  subsets. Production result schemas and registration remain forbidden until
  this matrix is fixed and exact.
- [ ] Give each of the 21 descriptors, and each physical variant within it, its
  exact completed set, stopped code/data set, and matrix-derived rejected
  subset. There is no common rejected set; mismatched producer/code/context,
  stop/data, or primary error is impossible in Rust and rejected by schema.
- [ ] Create the sole production
  `TaskResultEnvelope` as exactly
  `ReadOnlyTaskResultEnvelope | MutatingTaskResultEnvelope`. The read-only side
  physically forbids top-level `operationId`; the mutating side requires the
  request's exact value. Bind all generic change/warning/artifact/cache/evidence/
  data slots to the exact named schemas owned by Tasks 12-15; no placeholder
  payload record survives.
- [ ] Create the sole production
  `CurrentOperationRecord = OperationRecord<MutatingTaskResultEnvelope>` and
  bind its exact typed `operation: TaskOperationSelector` to the legal durable
  policy and matching terminal producer. Do not add duplicate
  `toolName`/`requestVariant` fields. Reject `policy: readOnly`, every physical
  selector variant mapped to `readOnly` (including one inside a mixed-policy
  tool), every operation/policy mismatch, every read-only terminal envelope,
  every operation/policy/terminal-envelope mismatch, and every terminal record
  whose `terminalEnvelope.operationId` differs from the record `operationId` or
  whose `terminalEnvelope.taskId` differs from `scope.taskId`. Require
  `startAttempt` scope exactly for `branched.start` and `task` scope for every
  other durable selector. Successful start additionally binds returned
  project/instance IDs, the created task record, and locator; early failed start
  requires neither identity.
  Invoke Task 11's lease and terminal-envelope digest validators before
  constructing the replay view. Classify each such stored mismatch or digest-
  validation failure as a retained corrupt candidate and block
  replay without fabricating `stateCorrupt` evidence yet: Task 17 supplies the
  committed expected schema digest, and Phase 2/Task 5B then maps either the
  exact retained bytes when readable or an `unavailable` observation for a
  missing/permission-denied object to the final `stateCorrupt` context. Derive `operation`
  from the validated closed request before registration and require replay to
  derive the same selector; never accept it as an extra caller field.
- [ ] Bind Task 5A's opaque candidate loader to `CurrentOperationRecord` only;
  only a fully typed/schema-valid current record may construct the replay view.
  Keep the normalized production schema/digest/catalog snapshots absent until
  Task 17.
- [ ] Define internal `BranchedToolResult` variants and projection into the
  serialized application envelope without command/stdout/stderr leaks.
- [ ] Every production result constructor takes the validated physical request
  and copies its exact `taskId`; no handler-facing API accepts an independent
  response task ID. Test all 53 variants plus cross-task substitution.
- [ ] Construct `commitCommentPolicyMismatch.mismatchKinds` and
  `integrationSetMismatch.mismatchKinds` only from their two authoritative typed
  preimage records, deriving every unequal component in canonical order. Create
  the two closed content-addressed mismatch-proof records and typed wire
  evidence refs; expose no constructor that accepts a caller-provided
  projection. Define the replay semantic validator that resolves and rehashes
  the immutable proof, matches both context digests and the projection, and
  never reconstructs historical observed policy state from a current profile.
  Phase 2 must invoke it before replaying a stored terminal. Standalone wire
  deserialization can prove only unequal opaque record digests plus non-empty/
  canonical kinds.
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
  tool and the exhaustive variant-policy manifest. This is the first task that
  generates the normalized fully bound `CurrentOperationRecord` schema,
  computes its expected digest, records the externally selected schema-catalog
  entry, and writes operation/lease/terminal/status storage snapshots; no digest
  is embedded in the stored record.
- [ ] Add an explicit ignored regeneration test guarded by an environment flag;
  ordinary tests never write.
- [ ] Assert exact count/name/error/policy sets, recursive closure, bounds,
  required fields, read-only durability exclusion, rejection of every physical
  operation selector whose variant policy is `readOnly`,
  operation/policy/terminal-envelope mismatches, absence of duplicate durable
  `toolName`/`requestVariant` fields, exact equality between record and terminal-
  envelope operation/task IDs, exact scope/container and scope/selector
  binding (including original-workspace-scoped start attempts), rejection of every
  substituted heartbeat/lease/terminal-envelope/mismatch-proof digest or proof
  kind/context binding, and raw
  path/process/secret field absence.
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
