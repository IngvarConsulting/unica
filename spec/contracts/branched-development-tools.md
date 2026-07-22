# Branched Development Tool Contract

## Status And Scope

This is the normative semantic request/response contract for the 21 public
tools in
[ADR-0012](../decisions/0012-safe-branched-development-for-1c-configuration-repositories.md).
The implementation publishes generated JSON Schemas and commits their exact
snapshots under
`tests/fixtures/branched_development/tool_schemas/<tool>.json`; those snapshots,
Rust request/result types, and contract tests become the executable schema
contract before any handler is registered. They use camelCase,
`additionalProperties: false` recursively, bounded strings/collections, and
`oneOf` tagged unions. No free-form object or untyped array is allowed.

The record notation below is normative for required fields and enums: `?`
means optional and `[]` means a typed collection. Fields not shown are rejected.
`cwd` is the one caller-supplied path: it selects the original project workspace
and is resolved by normal workspace policy. Designer executables, repository
locations, task roots, artifacts, checkpoints, and sandboxes come only from
local profile or registered IDs.

Layer-aware `unica.support.edit` is a separately owned general developer tool
required by the packaged workflow. Other general developer mutations,
including BSL writing, are package capabilities rather than global
prerequisites. None is counted among these 21 lifecycle tools.

The companion contract for general mutations is producer-neutral:

- Every general mutation operation advertised for `branchedTask`, regardless
  of tool name or change kind, returns the closed `BranchedChangeReceipt` plus
  `cacheImpact`; changed/no-change target, hash, event, and invalidation
  evidence follows the four closed receipt leaves defined below. A
  manual/combine decision can consume only the exact selectable changed receipt
  for its target. A BSL writer is optional: `unica.code.patch`, proposed by
  issue #73, is one possible implementation and is not a dependency of issue
  #137 or package release. If no compatible BSL writer is advertised, only a
  task or conflict that actually requires BSL text mutation stops before
  mutation; metadata/form/other typed tasks and the repository lifecycle remain
  available. The skill never falls back to shell or direct file editing.
- `unica.support.edit` accepts `mode: "layer"`, `layerId`, and exactly one of
  `{ operation: "setCapability", enabled }` or
  `{ operation: "setObjectState", objectId, state }`, where state is `locked`,
  or `editable`. `offSupport` is absent from the branched-task variant and is
  rejected. It supports an honest dry-run and returns a layer/object
  before/after receipt plus proof that all unrelated layers are byte-identical.
  Legacy broad `Capability` or unscoped UUID matching is rejected when a
  branched task is active.

## Common Types And Schema Bounds

- `TaskId`: 1-64 ASCII characters matching
  `[A-Za-z0-9][A-Za-z0-9._-]{0,63}`.
- `OriginalProjectCwd`: a 1-4096-Unicode-scalar non-empty string with no control
  characters. It is a workspace selector interpreted by the existing workspace
  path policy, not a persisted/canonical identity and never a task workspace,
  artifact, state-root, executable, or connection path. Resolution must prove
  that its local project configuration owns the requested `projectId`/task.
- `LocalProfileName`: 1-256 printable non-control Unicode scalar values. It is
  the exact key of a local branched-development profile and is distinct from
  `ProfileArtifactRefId`, which names an artifact entry inside that profile.
- `OperationId`: canonical UUID string.
- `ProjectId`: canonical UUID from local project configuration.
- `UnicaId`: canonical UUID allocated by Unica and scoped to one task. This is
  used by instance, workspace, infobase, artifact, probe, comparison, manifest,
  checkpoint, session, decision, verification, validation-receipt,
  support-gate, support-action, support-candidate-set,
  support-prerequisite-receipt, plan,
  integration-set, lock-set, operation-receipt, archive, and quarantine IDs.
- `MetadataObjectId`: canonical metadata UUID from the 1C configuration; Unica
  does not allocate it.
- `SupportLayerId`: 1-256 printable non-control Unicode characters, returned by
  support inspection and compared exactly.
- `ProfileArtifactRefId`: 1-128 ASCII characters matching
  `[A-Za-z0-9][A-Za-z0-9._-]{0,127}`; it names a validated profile entry, never
  a path.
- `SupportPreflightOutcome`: closed `ready`, `manualSupportRequired`,
  `vendorForbidsChanges`, or `supportPreflightInconclusive`.
- `SupportGateMismatchKind`: closed `candidateSetChanged`,
  `canonicalDeltaChanged`, `ordinaryResultChanged`, `supportGraphChanged`,
  `recoveryDistributionSetChanged`, `settingsChanged`, `sandboxResultChanged`,
  `capabilityRowChanged`, or
  `originalFingerprintChanged`.
- `SupportGateInputDigests`: closed `{ candidateSetDigest,
  canonicalDeltaDigest, ordinaryResultDigest, supportGraphDigest,
  supportRecoveryDistributionSetDigest,
  settingsDigest, sandboxResultDigest, capabilityRowDigest,
  originalFingerprintDigest }`.
- `ManualSupportTargetMode`: closed `reservedOriginal` or
  `separateWorkingInfobase`; it comes from the validated local profile and is
  bound into the authorization rather than selected ad hoc by a caller.
- `SupportRecoveryDisposition`: closed `restoreThenReauthorize`,
  `preserveExternalAndReauthorize`, or `restoreThenAbandon`. No immutable invalid version is retroactively accepted
  as the authorized prerequisite. `restoreThenReauthorize` inverses only this action's invalid
  deltas on top of every semantically partitioned valid routine, proven
  disjoint external-support, and disposition-preserved externally owned version
  in repository order,
  cancels the authorization, and returns to its bound `cancelledPhase` so a new
  correctly attributed action can be issued. `restoreThenAbandon` is mandatory
  after any `provenance: thisAuthorizedAction` unauthorized-content or
  off-support history and permanently forbids the
  task's successful integration path while preserving those same valid routine
  and externally owned support versions.
  `preserveExternalAndReauthorize` applies to a capability-proven wrong-actor/
  wrong-target version: it never inverses another actor's change, cancels this
  authorization, preserves the external baseline, and returns to the bound
  relevant-advance phase for a fresh distribution/preflight. An overlapping or
  unclassifiable external support delta remains a typed coordination conflict;
  it is never guessed or silently reversed.
  A versionless `originalNotClean` mismatch uses
  `restoreThenReauthorize`: it selectively restores the original to the fully
  classified repository baseline, cancels the non-terminal authorization, and
  returns to its bound cancelled/relevant-safe phase. It is not immutable action
  taint and never selects `restoreThenAbandon` by itself.
- `ManualWorkingInfobaseIdentity`: closed `{ computer, infobase, digest }`;
  both display fields are non-empty bounded history-visible values. The closed
  `ManualWorkingInfobaseIdentityDigestRecord` is `{ computer, infobase }`, and
  `digest == sha256(canonical(ManualWorkingInfobaseIdentityDigestRecord))`. It
  contains no connection string or credential.
- `ManualWorkingInfobaseBaseline`: closed `{ workingInfobaseIdentity,
  repositoryBaseCursor: RepositoryHistoryCursor,
  recordedObjectVersionMapDigest, baseFingerprint, currentFingerprint,
  currentEqualsBase: true, supportGraphDigest, baselineInspectionReceiptId,
  exclusiveLeaseCapabilityId, leaseReleasedVerified: true, baselineDigest }`.
  `ManualWorkingInfobaseBaselineDigestRecord` is the same closed record with
  only top-level `baselineDigest` removed, and `baselineDigest ==
  sha256(canonical(ManualWorkingInfobaseBaselineDigestRecord))`. It is captured
  under the service-held exclusive lease immediately before a separate-mode
  authorization is exposed and proves a clean starting IB.
- `ReservedOriginalTerminalizationProof`: closed `{
  reservedOriginalIdentityDigest, exclusiveLeaseCapabilityId,
  exclusiveLeaseReceiptId, exclusiveLeaseReleaseReceiptId,
  designerSessionClosedBeforeAcquisition: true,
  exclusiveConfigurationLeaseAcquired: true,
  leaseHeldThroughInspectionAndTerminalization: true,
  expectedRepositoryFingerprint, observedOriginalFingerprint,
  originalEqualsClassifiedRepositoryState: true,
  noUncommittedConfigurationDelta: true, leaseReleased: true,
  leaseReleaseVerified: true, proofDigest }`.
  `ReservedOriginalTerminalizationProofDigestRecord` is the same closed record
  with only top-level `proofDigest` removed, and `proofDigest ==
  sha256(canonical(ReservedOriginalTerminalizationProofDigestRecord))`. The
  capability receipt, not a user assertion or process-name snapshot, proves
  that no Designer/configuration
  session can mutate the reserved original from the guarded inspection through
  authorization consumption/cancellation/finalization. The acquisition/release receipt IDs name the exact
  capability-held lease window and cannot come from different attempts.
- `VendorChangeRestriction`: closed `changesAllowed`,
  `changesNotRecommended`, `changesForbidden`, `unknown`, or `notApplicable`.
- `CapabilityRowId`: 1-128 ASCII characters matching
  `[a-z0-9][a-z0-9._-]{0,127}`.
- `RepositoryVersion`: 1-128 printable non-control Unicode characters; it is
  opaque and never parsed, numerically compared, or lexically ordered by the
  domain. Repository-history order, immediate-successor claims, and contiguous
  coverage are supplied by an adapter under a capability-proven history API.
  Given that ordered evidence, the `RepositoryHistoryOrderResolver`-backed
  domain constructor rejects duplicate versions, endpoint disagreement, and
  cursor/partition digest disagreement; it does not manufacture continuity from
  the spelling of a version.
- `NormalizedUtcInstant`: one RFC 3339 UTC instant serialized with uppercase
  `T` and terminal `Z` as `YYYY-MM-DDTHH:MM:SS[.fraction]Z`. The calendar and
  clock value are parsed and validated. A zero fraction is omitted; a non-zero
  fraction has 1-9 digits and no trailing zero. Numeric offsets (including
  `+00:00`), lowercase `t`/`z`, spaces, redundant fractional zeroes, and leap
  seconds are rejected rather than normalized after durable input is accepted.
  This exact representation is used by canonical JSON and digest inputs.
- `RepositoryHistoryCursor`: closed `{ throughVersion: RepositoryVersion,
  historyPrefixDigest: Sha256 }`; it is the global immutable history scan
  position and is never used by itself as evidence that task-relevant content
  changed.
- `RelevantBaselineDigest`: `Sha256` over the exact task candidate/reference
  closure, its repository content/ownership, and the support root/layers on
  which that closure depends. A global history-cursor advance with the same
  recomputed digest is an unrelated advance only when the complete intervening
  partition is `unrelatedRoutine`. Any relevant/support/pre-arm-external/
  invalid/corrective
  entry invalidates the old baseline even when later history restores the same
  net digest; change-then-revert is not erased.
- `RoutineRepositoryVersionClassificationEvidence`: closed `{
  repositoryVersion: RepositoryVersion, relevance: unrelated | relevant,
  repositoryActor: RepositoryActorIdentity | null, rootDeltaDigest: Sha256,
  contentDeltaDigest: Sha256,
  supportTransitionsDigest: CanonicalEmptyDeltaDigest,
  supportGraphUnchanged: true, classificationDigest: Sha256 }`.
  `RoutineRepositoryVersionClassificationEvidenceDigestRecord` is that exact
  closed record with only top-level `classificationDigest` removed, preserving
  the required explicit nullable actor. `classificationDigest ==
  sha256(canonical(RoutineRepositoryVersionClassificationEvidenceDigestRecord))`.
  This is the
  capability-proven source for a routine partition entry when no overlapping
  `SupportPrerequisiteVersionObservation` exists, including an unrelated
  post-commit interleaving; nullable actor evidence remains an explicit `null`.
- `NonConflictingConcurrentEvidence`: closed `{ repositoryVersion, reason:
  harmlessNonBlockingReferenceExpansion,
  atomicCommitSafetyCapabilityId: CapabilityRowId, lockedTargetSetDigest,
  changedObjectSetDigest, beforeReferenceClosureDigest,
  afterReferenceClosureDigest, addedReferenceEdgeSetDigest,
  closureDeltaOnlyAddsNonBlockingReferences: true,
  disjointFromIntegrationContent: true, supportGraphUnchanged: true,
  validationInputsUnaffected: true, rootUnchanged: true,
  lockedTargetsUnchanged: true, blocksApprovedDeletion: false,
  evidenceDigest }`. All seven safety literals are derived from the complete
  capability-proven version delta/reference/validation-input scan.
  `NonConflictingConcurrentEvidenceDigestRecord` is that exact closed record
  with only top-level `evidenceDigest` removed; `evidenceDigest ==
  sha256(canonical(NonConflictingConcurrentEvidenceDigestRecord))`.
- `RepositoryHistorySourceEvidenceRef`: closed `{
  sourceKind: contentAddressed, evidenceKind: routineClassification |
  supportPrerequisiteObservation | nonConflictingConcurrent,
  evidenceDigest: Sha256 }`. The tuple `(evidenceKind, evidenceDigest)` is the
  one mandatory lookup key; there is no inline-only or implementation-selected
  alternative. A deterministic content-addressed evidence resolver must load
  the exact typed record selected by `evidenceKind`, recompute its digest, and
  reject a missing record, wrong type, digest mismatch, or non-canonical record.
  Task 7 registers only `RoutineRepositoryVersionClassificationEvidence` and
  `NonConflictingConcurrentEvidence`; Task 8 registers
  `SupportPrerequisiteVersionObservation` after that type exists. An unresolved
  `supportPrerequisiteObservation` reference is unusable rather than partially
  trusted.
- `EvidenceSourceRegistry` is the internal typed registry of those loaders,
  rehashers, and classification mappers. `EvidenceSourceRegistryEntry` is the
  closed `{ evidenceKind, evidenceSchemaDigest: Sha256,
  digestRecordSchemaDigest: Sha256, loaderRevisionDigest: Sha256,
  classificationMapperRevisionDigest: Sha256 }`. The non-empty entry list is
  unique and ordered by evidence-kind declaration order. Each digest is a
  generated and committed contract artifact for, respectively, the exact typed
  evidence `$defs` schema, its exact digest-input-record schema, the typed loader
  revision, and the classification/semantic-mapping revision. The two schema
  digests are `sha256(canonical(schema))` over the exact standalone Draft
  2020-12 document emitted by the contract schema factory for the named evidence
  or digest-record type, including its `$schema`, title, and every reachable
  `$defs`, with no stripping, rewriting, or second normalization pass. Their
  lowercase digest constants are committed beside recomputation tests, so a
  schema-generator or reachable-definition change fails review rather than
  silently changing the active registry.

  `EvidenceLoaderRevisionDigestRecord` is the closed internal canonical record
  `{ loaderKind: contentAddressedTypedEvidence, evidenceKind,
  validationChecks: [lookupByEvidenceKindAndDigest, requireSingleRecord,
  strictTypedDecode, requireCanonicalIJson, projectNamedDigestRecord,
  recomputeAndMatchDigest] }`; the tuple order is exact and its tail is closed.
  `loaderRevisionDigest ==
  sha256(canonical(EvidenceLoaderRevisionDigestRecord))`.
  `EvidenceClassificationMapperRevisionDigestRecord` is the closed tagged
  internal record `{ mapperKind: repositoryHistoryPartitionClassification,
  evidenceKind, validationChecks: [repositoryVersionMatch,
  sourceClassificationMatch, semanticDeltaProjectionMatch], mappings }`, where
  every mapping row is the closed `{ sourceCase, partitionClassification,
  rootDeltaDigestProjection, contentDeltaDigestProjection,
  classificationDigestProjection,
  externalSupportDisjointnessDigestProjection,
  correctiveInstructionDigestProjection,
  nonConflictingConcurrentEvidenceDigestProjection }` record. Task 7 has
  exactly two physical leaves with fixed-tuple `mappings` and no open tail:

  - `routineClassification` has exactly the `unrelated` row followed by the
    `relevant` row. Their `partitionClassification` values are respectively
    `unrelatedRoutine` and `relevantRoutine`; both rows have literal projections
    `copyRootDeltaDigest`, `copyContentDeltaDigest`,
    `copyClassificationDigest`, `explicitNull`, `explicitNull`, and
    `explicitNull` in the six projection fields above;
  - `nonConflictingConcurrent` has exactly one row with `sourceCase:
    harmlessNonBlockingReferenceExpansion`, `partitionClassification:
    nonConflictingConcurrent`, and literal projections `explicitNull`,
    `explicitNull`, `explicitNull`, `explicitNull`, `explicitNull`, and
    `copyEvidenceDigest`.

  Task 8 adds `supportPrerequisiteObservation` with exactly six rows in the
  fixed tuple order `routineUnrelated`, `routineRelevant`, `authorized`,
  `externalSupport`, `preArmExternal`, `invalid`. Their partition
  classifications are respectively `unrelatedRoutine`, `relevantRoutine`,
  `authorizedSupport`, `externalSupport`, `preArmExternal`, and `invalid`.
  Every row copies root, content, and classification digests and uses
  `explicitNull` for the corrective and non-conflicting slots. The
  `externalSupport` row alone uses
  `copyExternalSupportDisjointnessDigest` for the external-support slot; every
  other row uses `explicitNull` there. The `invalid` row's copy projections
  preserve the observation's explicit `null` root/content values for the
  unattributed leaf. The structurally representable `corrective` observation
  has no mapper row until Task 9 binds it to an exact historical instruction.

  Task 9 replaces that six-row mapper with exactly eight rows in fixed order:
  `routineUnrelated`, `routineRelevant`, `authorized`, `externalSupport`,
  `preArmExternal`, `actionCorrection`, `externalConflictCorrection`,
  `invalid`. The two new rows both select `corrective`, copy root/content/
  classification digests, and keep external-support/non-conflicting slots
  explicitly null. `actionCorrection` uses
  `copyCorrectiveInstructionDigest`; `externalConflictCorrection` uses
  `copySupportConflictInstructionDigest`. The distinct projection literals are
  part of the mapper preimage, so the two historical instruction kinds cannot
  be interchanged.

  Those named member strings and literal values are the canonical preimage;
  `null` is not used inside the descriptor row because `explicitNull` describes
  the mapper's output policy.
  `classificationMapperRevisionDigest ==
  sha256(canonical(EvidenceClassificationMapperRevisionDigestRecord))`.
  These descriptor records and their lowercase digest constants are committed
  and recomputed by tests; changing a check, legal mapping, mapping order, or
  digest-slot projection changes the registry. They contain no arbitrary version
  strings, source paths, function pointers, debug names, or invented
  profile/platform capability row. When a loader's evidence genuinely depends on
  an existing authoritative platform capability row, that row remains inside
  the typed evidence itself; the code registry does not manufacture a generic
  loader `CapabilityRowId`.
  `EvidenceSourceRegistryDigestRecord` is the closed `{ entries:
  EvidenceSourceRegistryEntry[] }`, and `registryDigest ==
  sha256(canonical(EvidenceSourceRegistryDigestRecord))`. Task 7's registry
  contains exactly `routineClassification` and `nonConflictingConcurrent`; Task
  8 extends that same registry with `supportPrerequisiteObservation`, changing
  `registryDigest` and invalidating every older index proof. When Task 9 enables
  the two previously rejected `corrective` mappings, it changes the affected
  support-observation entry's `classificationMapperRevisionDigest`, recomputes
  `registryDigest`, and invalidates every Task 8 index proof. Its
  `evidenceSchemaDigest`, `digestRecordSchemaDigest`, and `loaderRevisionDigest`
  remain byte-identical unless that schema/loader actually changes; mapping-only
  enablement cannot falsify those revisions. `taskCommit` is not an evidence-
  source kind and remains owned by its enclosing Task 13 constructor.
  The capability-backed, version-indexed `EvidenceSourceIndex` produces one
  internal, non-`Deserialize` `EvidenceSourceIndexProof` per queried
  `repositoryVersion`: closed `{ repositoryVersion, registryDigest,
  sourceIndexReceiptId: UnicaId, availability: EvidenceSourceAvailability[],
  proofDigest }`.
  `EvidenceSourceAvailability` is the closed tagged `oneOf` of `available {
  evidenceKind, state: available, sourceEvidenceRef:
  RepositoryHistorySourceEvidenceRef }` or `absent { evidenceKind, state:
  absent }`. The availability list has exactly one row for every active registry
  entry and no other row; for every index `i`,
  `availability[i].evidenceKind == registry.entries[i].evidenceKind`, so its byte-
  for-byte order is exactly the active evidence-kind declaration order. A
  reordered row set is invalid and also changes `proofDigest`. An available
  row's ref has the same `evidenceKind`. Each available row identifies exactly
  one canonical ref. If
  the index observes multiple refs for a kind, cannot establish absence, or
  cannot cover the active registry, it produces no valid proof.
  `EvidenceSourceIndexProofDigestRecord` is the same closed record with only
  top-level `proofDigest` removed, and `proofDigest ==
  sha256(canonical(EvidenceSourceIndexProofDigestRecord))`. The proof is adapter
  evidence, not caller JSON; a proof from an older registry digest cannot omit a
  newly registered source kind.
- `RepositoryHistoryPartition` is the external schema name for the closed wire
  record `{ fromExclusive: RepositoryHistoryCursor, throughInclusive:
  RepositoryHistoryCursor, entries: RepositoryHistoryPartitionEntry[],
  partitionDigest }`. Ordinary Rust deserialization produces only
  `UnvalidatedRepositoryHistoryPartition`; that closed DTO has the same wire
  fields but has no domain methods and cannot enter status, gate, merge, commit,
  recovery, or other control flow. `entries` is empty if and only if
  `fromExclusive` and `throughInclusive` are byte-identical; differing endpoints
  require a non-empty list. Each entry is closed `{
  repositoryVersion, classification, semanticDeltaDigest,
  sourceEvidenceRef?: RepositoryHistorySourceEvidenceRef,
  nonConflictingConcurrentEvidence?: NonConflictingConcurrentEvidence }`, and
  `classification` is `unrelatedRoutine`, `relevantRoutine`,
  `authorizedSupport`, `externalSupport`, `preArmExternal`, `invalid`,
  `corrective`, `nonConflictingConcurrent`, or `taskCommit`.
  `sourceEvidenceRef` is required for every non-`taskCommit` entry and absent
  exactly for `taskCommit`; `taskCommit` also forbids inline concurrent evidence.
  Its evidence-kind mapping is closed:

  - `routineClassification` is legal only for `unrelatedRoutine` or
    `relevantRoutine` and resolves
    `RoutineRepositoryVersionClassificationEvidence.classificationDigest`;
  - `supportPrerequisiteObservation` is legal for an observation-backed routine,
    `authorizedSupport`, `externalSupport`, `preArmExternal`, `invalid`, or
    `corrective` entry and resolves
    `SupportPrerequisiteVersionObservation.classificationDigest`;
  - `nonConflictingConcurrent` is legal only for that same partition
    classification and resolves
    `NonConflictingConcurrentEvidence.evidenceDigest`.

  `nonConflictingConcurrentEvidence` is required exactly for
  `nonConflictingConcurrent`, is absent otherwise, and must be byte-identical to
  the typed content-addressed record resolved by its mandatory reference. The
  inline copy is never a substitute for lookup. Every later audit resolves and
  revalidates the same typed record; hash presence alone is insufficient.
  A capability-backed `RepositoryHistoryOrderResolver` consumes the unvalidated
  DTO, the exact resolved source records, one authoritative
  `EvidenceSourceIndexProof` for every non-`taskCommit` entry, and internal typed
  `RepositoryHistoryOrderEvidence` from the proven history adapter. For an empty
  DTO the resolver requires byte-identical endpoints and `entries: []`; there is
  no immediate-successor claim to prove. For a non-empty DTO the evidence proves
  the immediate successor after `fromExclusive`, every following successor,
  complete coverage through `throughInclusive`, and the exact ordered version
  sequence; it is not caller JSON. The resolver verifies the applicable case,
  rejects duplicates, endpoint/emptiness/order/coverage disagreement,
  revalidates source/classification/version mappings and all semantic/partition
  digests, then constructs `ValidatedRepositoryHistoryPartition`. That domain
  type has no `Deserialize` implementation and serializes back to the same
  closed wire schema. No implementation compares or parses version strings to
  replace the resolver.
  In support-recovery ranges, observation `authorized` maps to
  `authorizedSupport`; routine relevance maps
  to `unrelatedRoutine`/`relevantRoutine`; the remaining discriminator names
  map directly, and neither commit-only classification is legal. In a
  post-commit range, `taskCommit` names exactly the task's own version;
  `nonConflictingConcurrent` requires capability evidence that the entry changed
  no integration content, validation input, support graph, locked target/root,
  and introduced only reference-closure expansion that cannot block an approved
  deletion. The inline `nonConflictingConcurrentEvidence` field is required
  exactly for that classification, its version/capability equal the
  entry/enclosing atomic-safety guard, and it is absent for every other
  classification. For every entry except `taskCommit`,
  `semanticDeltaDigest` is exactly
  `sha256(canonical(RepositorySemanticDeltaDigestRecord))`, where the closed
  record is `{ repositoryVersion, partitionClassification,
  rootDeltaDigest: Sha256 | null, contentDeltaDigest: Sha256 | null,
  classificationDigest: Sha256 | null,
  externalSupportDisjointnessDigest: Sha256 | null,
  correctiveInstructionDigest: Sha256 | null,
  nonConflictingConcurrentEvidenceDigest: Sha256 | null }`. All six digest
  members after `partitionClassification` are physically present, so an unavailable or
  inapplicable input is JSON `null`, never omitted. Their exact mapping is:

  - `unrelatedRoutine`, `relevantRoutine`, `authorizedSupport`, and
    `preArmExternal` copy the matching observation's root, content, and
    classification digests; the remaining three evidence slots are `null`;
  - `externalSupport` additionally copies the observation's
    `externalSupportDisjointnessDigest`; both corrective/non-conflicting slots
    are `null`;
  - `invalid` copies the observation's root/content values, preserving explicit
    `null` for unavailable unattributed deltas, and its classification digest;
    the remaining three slots are `null`;
  - `corrective` copies the observation's root, content, and classification
    digests. Its `correctiveInstructionDigest` slot is the action-correction
    variant's field of that name or the external-conflict-correction variant's
    `supportConflictInstructionDigest`; the external-support and
    non-conflicting slots are `null`;
  - `nonConflictingConcurrent` sets the root, content, classification,
    external-support, and corrective slots to `null`, and sets
    `nonConflictingConcurrentEvidenceDigest` to the entry evidence's exact
    `evidenceDigest`.

  Source selection is authoritative and version-indexed, never an implementation
  search for whichever record happens to exist. The total active-kind precedence
  is `supportPrerequisiteObservation > nonConflictingConcurrent >
  routineClassification`; `taskCommit` remains outside this selection path. The
  index proof's `repositoryVersion` equals the entry version, its
  `registryDigest` equals the active registry, and the entry's
  `sourceEvidenceRef` must byte-equal the uniquely selected available ref.
  `supportPrerequisiteObservation` is selected whenever available. A selected
  `nonConflictingConcurrent` requires every active higher-precedence row
  explicitly `absent` (the support-observation row after Task 8 registers it);
  its inline copy equals that exact ref. A selected `routineClassification`
  likewise requires every active higher-precedence row explicitly `absent`: in
  Task 7 that is `nonConflictingConcurrent`, while after Task 8 it is both
  `supportPrerequisiteObservation` and `nonConflictingConcurrent`. The active
  `registryDigest`, rather than a fabricated absent row for an unregistered kind,
  proves which set applies. An available higher-precedence source whose typed
  classification does not match the partition entry is validation failure,
  never permission to fall back to a lower kind. Every observation-backed class
  requires the exact available support-observation ref. A missing registered-
  kind row, missing legal source, multiple refs for one kind, stale/wrong-
  registry proof, version/kind/ref mismatch, or any remaining ambiguous
  selection produces no validated partition. The generic validated constructor
  verifies the selected source's
  repository version and classification mapping and recomputes
  `semanticDeltaDigest` from it. It never invents digest inputs from an opaque
  `RepositoryVersion`, accepts a caller-supplied semantic hash alone, or treats
  index absence as an unproven filesystem/store search result.

  The wire schema may name `taskCommit`, but Task 7's generic constructor rejects
  every such entry: no partially validated partition is produced. `taskCommit`
  does not use `RepositorySemanticDeltaDigestRecord`. Task 13 defines the closed
  `CommittedRepositoryObject` element type used by `CommitData.committedObjects`
  and owns the only crate-private task-commit partition constructor. That
  constructor receives the enclosing validated `CommitData` inputs, uses the
  same history-order resolver for the entire interval and the authoritative
  source-index proof plus typed source resolver for every other entry, requires
  exactly one task version, and
  recomputes its `semanticDeltaDigest == committedObjectsDigest ==
  sha256(canonical(CommittedObjectsDigestRecord))` after validating the exact
  committed-object projection defined by `CommitData`. No generic/public
  constructor or raw deserialization can bypass that enclosing validation. No
  version is inferred from a localized diagnostic. A root
  guard excludes new root/support versions only; it never claims to serialize
  commits of unrelated development objects.
  `partitionDigest == sha256(canonical({ fromExclusive, throughInclusive,
  entries }))`; endpoint or entry substitution therefore changes the digest.
- `DeferredRepositoryAdvance`: closed tagged `oneOf` of `classified {
  state: classified, fromCursor: RepositoryHistoryCursor, firstObservedVersion:
  RepositoryVersion, classification: authorizedSupport | invalid | corrective,
  semanticDeltaDigest, requiredNextMode: routine, observationDigest }` or
  `unclassified { state: unclassified, fromCursor: RepositoryHistoryCursor,
  firstObservedVersion: RepositoryVersion, missingEvidenceKinds:
  SupportMissingEvidenceKind[], requiredNextMode: routine, observationDigest }`,
  or `coverageUnknown { state: coverageUnknown, fromCursor:
  RepositoryHistoryCursor, missingEvidenceKinds:
  [repositoryHistoryCoverageIncomplete], requiredNextMode: routine,
  observationDigest }`. It is not part of the preceding terminal receipt.
  `firstObservedVersion` exists only in the first two variants and is then
  capability-proven to be the immediate history successor of `fromCursor`.
  A `coverageUnknown` record makes no version claim. Each observation digest
  covers every field except itself. An `unclassified` missing-evidence list is
  non-empty and canonical and excludes `repositoryHistoryCoverageIncomplete`;
  its exact successor is known even though its semantic classification is not.
  This makes a post-terminal
  root/support tail reachable without reopening the already
  consumed/cancelled authorization: the next authoritative call must be
  `repository.update(mode="routine")`, starting exactly at `fromCursor`.
- `DeferredRepositoryAdvanceConsumptionReceipt`: closed `{
  consumptionReceiptId: UnicaId, terminalReceiptId: UnicaId,
  advanceObservationDigest: Sha256, routineUpdateReceiptId: UnicaId,
  resolvedHistoryPartitionDigest: Sha256, resultingPhase: TaskPhase,
  receiptDigest }`.
  It is written atomically only after the approved routine apply reproduces the
  deferred observation, completes the no-force selective refresh, and verifies
  its postcondition. `receiptDigest ==
  sha256(canonical(receipt-without-receiptDigest))`. Task 7 closes and hashes
  the phase type but does not guess a narrower mode-specific set. The later
  enclosing `RepositoryUpdateData` constructor requires this value to be
  byte-identical to its own validated `resultingPhase` and enforces that update
  mode's phase semantics.
- `SupportGateHistoryEvidence`: closed `{ gateObservedCursor:
  RepositoryHistoryCursor, classifiedThroughCursor: RepositoryHistoryCursor,
  partition: RepositoryHistoryPartition,
  relevantBaselineDigest: RelevantBaselineDigest, evidenceDigest }`. For a
  reusable current gate, every entry is `unrelatedRoutine`. The evidence is
  valid only when `partition.fromExclusive == gateObservedCursor` and
  `partition.throughInclusive == classifiedThroughCursor`.
  `SupportGateHistoryEvidenceDigestRecord` is the closed
  `{ gateObservedCursor, classifiedThroughCursor, partitionDigest,
  relevantBaselineDigest }`, with `partitionDigest` copied from `partition`;
  `evidenceDigest == sha256(canonical(SupportGateHistoryEvidenceDigestRecord))`.
  The recomputed relevant-baseline digest must equal the current gate
  baseline after applying that partition. The evidence is
  immutable and is carried through main verification, plan, lock, original
  merge receipt, and consumed-gate lineage; a cursor advance without it cannot
  use the old gate.
- `PostMergeHistoryGuardEvidence`: closed `{ mergeReceiptCursor:
  RepositoryHistoryCursor, classifiedThroughCursor: RepositoryHistoryCursor,
  partition: RepositoryHistoryPartition, recomputedReferenceClosureDigest,
  relevantTailAbsent: true, atomicCommitSafetyCapabilityId:
  CapabilityRowId, evidenceDigest }`. It proves every intervening version is
  unrelated to the integration/reference/support closure. Commit apply supplies
  this pre-effect evidence to a capability-proven atomic repository safety
  boundary. The real-fixture capability proves that locks plus the platform's
  no-force atomic commit validation reject, without partial task commit, every
  concurrent change to a locked target/root or new reference that would block
  an approved deletion. Other concurrently committable closure expansion is
  retained as capability-proven `nonConflictingConcurrent`; it is not falsely
  required to fail. A pre-intent referrer, or a concurrent referrer that changes
  a locked target/root or blocks an approved deletion, starts no task-content
  commit and enters restore/unlock recovery; every other post-boundary entry is
  classified explicitly rather than being called conflicting by default.
  It is valid only when `partition.fromExclusive == mergeReceiptCursor` and
  `partition.throughInclusive == classifiedThroughCursor`.
  `PostMergeHistoryGuardEvidenceDigestRecord` is the closed
  `{ mergeReceiptCursor, classifiedThroughCursor, partitionDigest,
  recomputedReferenceClosureDigest, relevantTailAbsent,
  atomicCommitSafetyCapabilityId }`, with the partition digest copied from the
  nested partition and `relevantTailAbsent: true` retained literally;
  `evidenceDigest ==
  sha256(canonical(PostMergeHistoryGuardEvidenceDigestRecord))`. No other range
  can be substituted.
- `RepositoryTargetKind`: the closed `configurationRoot` or
  `developmentObject` repository-identity discriminator. It is distinct from
  `TargetKind` (`task` or `original`), which chooses a merge/apply destination
  and is never accepted as a repository target discriminator.
- `RepositoryTargetState`: closed tagged `oneOf` of `rootPresent { targetKind:
  configurationRoot, state: present, repositoryVersion, targetFingerprint }`,
  `objectPresent { targetKind: developmentObject, state: present, objectId, repositoryVersion,
  targetFingerprint }`, or `objectAbsent { targetKind: developmentObject,
  state: absent, objectId, absenceEstablishedAtVersion: RepositoryVersion, expectedAbsent:
  true }`. Repository-target canonical order places a `configurationRoot`, when
  present, uniquely first, followed by `developmentObject` entries in
  ascending lexicographic order of their canonical lowercase `MetadataObjectId`
  strings. Every target-state/planned-change/lock-target collection uses that
  order and rejects duplicate identities; the root can never be absent.
- `RepositoryTargetIdentity`: closed tagged `oneOf` of `configurationRoot {
  targetKind: configurationRoot }` or `developmentObject { targetKind:
  developmentObject, objectId: MetadataObjectId }`.
- `RepositoryTargetDisplay`: 1-512 printable non-control Unicode characters,
  redacted before persistence and used only for presentation. It never
  participates in target identity, equality, ordering, lock selection, replay
  selection, or conflict classification.
- `RepositoryPlannedChange`: closed tagged `oneOf` of `rootModify { targetKind:
  configurationRoot, action: modify, objectDisplay, repositoryVersion,
  targetFingerprint, relevance: relevant | unrelated }`, `objectPresent {
  targetKind: developmentObject, objectId, objectDisplay, action: add | modify,
  repositoryVersion, targetFingerprint, relevance: relevant | unrelated }`, or
  `objectAbsent { targetKind: developmentObject, objectId, objectDisplay,
  action: delete, deletionRepositoryVersion, expectedAbsent: true, relevance:
  relevant | unrelated }`. Lists of planned changes are canonical and unique by
  target identity; their version/fingerprint is the final folded state, never a
  duplicate per-history-event entry.
- `RepositoryUpdateLockReason`: closed `supportGraphGuard`, `updateTarget`,
  `parentClosure`, `referenceClosure`, or `structuralClosure`. A reason list is
  non-empty, duplicate-free, and ordered exactly as that declaration; it is not
  caller order or lexical enum order.
- `RepositoryUpdateLockTarget`: closed tagged `oneOf` of
  `configurationRoot { targetKind: configurationRoot, objectDisplay, reasons:
  RepositoryUpdateLockReason[] }` or `developmentObject { targetKind:
  developmentObject, objectId, objectDisplay, reasons:
  RepositoryUpdateLockReason[] }`. The root variant rejects `objectId`; the
  development-object variant requires its canonical metadata ID.
  The root is unique and first, reasons are non-empty/canonical, and following
  entries are the exact existing target/parent/referrer closure; an added or
  absent object is represented by its existing closure rather than a fabricated
  lock.
- `SelectiveRepositoryUpdatePlan`: closed `{ scope: routinePlannedObjects |
  supportRoot | recoveryFinalization, plannedTargets:
  RepositoryTargetState[], lockTargets: RepositoryUpdateLockTarget[],
  expectedTargetRevisionMapDigest,
  selectiveObjectsCapabilityId: CapabilityRowId,
  structuralConfirmationRequired,
  structuralCapabilityRowId?: CapabilityRowId, planDigest }`. It describes
  only the explicit `-Objects` set and expected per-target repository state;
  it is never an instruction to update a bound configuration through a global
  repository version. `expectedTargetRevisionMapDigest ==
  sha256(canonical(plannedTargets))`; every planned target has complete coverage
  by the root/target/structural closure in `lockTargets`. Cancellation may use an
  empty planned set when the root did not change, while retaining its root guard.
  `planDigest == sha256(canonical(plan-without-planDigest))`.
  `structuralConfirmationRequired` is true exactly when the enclosing approved
  routine/recovery change set contains an add/delete that the proven platform
  path requires the repository-update structural confirmation to receive. Its
  capability row is then required; otherwise it is absent. `supportRoot` always
  uses `false` and has no structural capability field. Neither value is caller
  selectable.
- `SelectiveRepositoryUpdateProof`: closed `{ planDigest: Sha256,
  guardReceiptId: UnicaId,
  plannedTargets,
  appliedTargets: RepositoryTargetState[], expectedTargetRevisionMapDigest,
  appliedTargetRevisionMapDigest,
  lockTargets: RepositoryUpdateLockTarget[],
  acquiredRootFirst: RepositoryUpdateLockTarget[],
  releasedInReverseOrder: RepositoryUpdateLockTarget[],
  releaseVerified: true,
  beforeOriginalTargetFingerprintMapDigest,
  updatePerformed, updateEffectReceiptId?, updateEffectReceiptDigest?,
  structuralConfirmationUsed,
  structuralCapabilityRowId?: CapabilityRowId,
  verifiedOriginalTargetFingerprintDigest, observedBeforeCursor:
  RepositoryHistoryCursor, observedAfterCursor: RepositoryHistoryCursor,
  selectiveObjectsCapabilityId: CapabilityRowId, proofDigest }`. It proves the
  `plannedTargets` and the expected digest equal the approved plan byte-for-byte;
  `appliedTargetRevisionMapDigest == sha256(canonical(appliedTargets))`, and a
  completed proof requires `appliedTargets == plannedTargets` byte-for-byte.
  It also requires `lockTargets` to equal the approved plan's list,
  `acquiredRootFirst == lockTargets` byte-for-byte, and
  `releasedInReverseOrder` to equal the exact reverse of `lockTargets`; partial
  acquisition belongs to stopped/recovery evidence, not this completed proof.
  Any target drift observed after locking releases the set and returns a stale
  plan before update; a post-update mismatch is an unknown/capability-breach
  recovery, not a completed proof. It proves the
  resulting original matches the repository state of every applied target
  while the plan's exact root-first target closure was held, then proves reverse
  release. Added/deleted targets are covered by their exact existing structural
  closure and capability rather than a fabricated lock on an absent object. It
  makes no claim that unselected objects
  equal a later global head. `updatePerformed: false` is legal only when the
  selected target map was already exact (including the empty cancellation set);
  then both update-receipt fields are absent and
  `structuralConfirmationUsed` is false. When an update runs, both receipt
  fields are required and identify its immutable journal effect receipt; they
  are absent together otherwise. The invocation is the
  exact selective no-force invocation if the plan's structural flag is false.
  The proof repeats the plan's structural capability row iff that flag is true,
  and `structuralConfirmationUsed == updatePerformed &&
  structuralConfirmationRequired`. Thus a required structural update uses the
  adapter-derived exact repository-update confirmation; an already-exact map
  records no invocation. For pre-arm cancellation finalization,
  `alreadyExact` requires the immutable prior-operation guarded evidence
  defined below; a later matching recheck of an earlier non-empty stage is only
  a freshness result and does not substitute for that evidence, so its approved
  `perform` action still runs and records its receipt. This exception never applies to `supportRoot` or to any
  merge/commit/unlock operation.
  `SelectiveRepositoryUpdateProofDigestRecord` is the closed
  `{ planDigest, guardReceiptId, plannedTargets, appliedTargets,
  expectedTargetRevisionMapDigest, appliedTargetRevisionMapDigest, lockTargets,
  acquiredRootFirst, releasedInReverseOrder, releaseVerified,
  beforeOriginalTargetFingerprintMapDigest, updatePerformed,
  updateEffectReceiptId?, updateEffectReceiptDigest?,
  structuralConfirmationUsed, structuralCapabilityRowId?,
  verifiedOriginalTargetFingerprintDigest, observedBeforeCursor,
  observedAfterCursor, selectiveObjectsCapabilityId }` with the same exact
  conditional-presence rules as the proof; it is precisely the proof with only
  top-level `proofDigest` removed. `proofDigest ==
  sha256(canonical(SelectiveRepositoryUpdateProofDigestRecord))`.
  A read-update-read capability fixture must prove the mapping or mutation is
  disabled.
  For support reconciliation/cancellation, including no-arming cancellation
  recovery, `guardReceiptId` equals the same
  `SupportRootLockProof.guardReceiptId`; for armed support recovery finalization
  it equals the enclosing `SupportRecoveryGuardProof.guardReceiptId`. Routine refresh records
  its own temporary update-guard receipt. No proof splice across lock windows is
  schema-valid.
- `OriginalCleanRefreshProof`: closed `{ expectedOriginalFingerprint,
  observedOriginalFingerprint, observedHistoryCursor:
  RepositoryHistoryCursor, repositoryCleanAtObservedCursor: true,
  taskMergeStarted: false, capabilityRowId: CapabilityRowId, proofDigest }`.
  `OriginalCleanRefreshProofDigestRecord` is the closed
  `{ expectedOriginalFingerprint, observedOriginalFingerprint,
  observedHistoryCursor, repositoryCleanAtObservedCursor, taskMergeStarted,
  capabilityRowId }`, retaining the two literal booleans;
  `proofDigest == sha256(canonical(OriginalCleanRefreshProofDigestRecord))`.
  Only this evidence may classify a clean out-of-band refresh as
  `originalFingerprintChanged`; an unowned, local, or unclassified original
  delta enters recovery instead of merely staling a support gate.
- `RepositoryOwnerIdentity`: closed `{ username, computer, infobase,
  lockedAt }`; `username` is a proven 1-256 printable non-control Unicode-scalar
  repository username. `computer` and `infobase` are each explicitly `null` or
  1-256 printable non-control Unicode scalars, and `lockedAt` is explicitly
  `null` or `NormalizedUtcInstant`; none of the four members is omitted. When no
  username is proven, the enclosing owner field is `null`; diagnostics never
  infer one.
- `RepositoryActorIdentity`: closed `{ username, computer, infobase }` with the
  same 1-256 proven repository-username bound and explicit `null` or 1-256
  printable non-control Unicode scalars for the other fields. It identifies a history/commit actor without
  inference. Presence/equality requirements depend on the authorized manual
  target mode.
- `Sha256`: 64 lowercase hexadecimal characters.
- `canonical(value)` is the RFC 8785 JSON Canonicalization Scheme (JCS) UTF-8
  octet sequence of a schema-valid I-JSON value. Object members use RFC 8785
  ordering; array order, an explicit `null`, and the exact Unicode scalar value
  are preserved. It adds no whitespace, BOM, trailing newline, or Unicode
  normalization. Duplicate names, lone surrogates, non-finite or out-of-range
  numbers, and every other non-I-JSON input fail before hashing.
  `sha256(bytes)` is lowercase 64-hex. `record-without-x[-and-y]` removes only
  the named top-level member(s): an absent member remains absent and an
  explicit `null` remains present. This rule applies only to JSON-derived
  contract digests, never to file/artifact bytes or topology-identity hashes.
  Every production JSON-derived contract digest uses the same typed,
  fail-closed JCS implementation; canonicalization/validation failure is an
  error and no contract type may fall back to ordinary Serde text, debug output,
  a local hasher, or a second canonicalizer. RFC 8785 serializer conformance is
  tested separately with the standard number-format vectors, including
  `1E30 -> 1e+30`. The contract-digest helper intentionally rejects that input:
  this contract's stricter I-JSON profile rejects every integer-valued number
  whose magnitude exceeds `2^53 - 1`, regardless of exponent spelling. A raw
  canonicalizer vector therefore proves formatting only and does not claim that
  the same value is admissible contract data.
- `OperationInputDigestRecord`: closed `{ digestKind:
  branchedOperationInputV1, toolName: TaskOperationToolName,
  executionPolicy: DurableExecutionPolicy, request }`, where `request` is the complete
  schema-valid tagged request and excludes only its top-level `operationId`.
  `canonicalInputDigest == sha256(canonical(OperationInputDigestRecord))`.
  Thus the same request payload under another tool or execution policy has a
  different digest and cannot replay.
- `CanonicalEmptyDeltaDigest`: the literal
  `sha256(canonical([]))` =
  `4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945`;
  it is a closed value type, not an arbitrary `Sha256`, and is used when a typed
  semantic delta is proven empty rather than omitted.
- `DigestApproval`: `{ "digest": Sha256, "decision": "apply" }`.
- `OwnedTargetLocator`: `{ projectId: ProjectId, instanceId: UnicaId, role }`,
  where `role` is `instanceRoot`, `taskInfobase`, `taskWorkspace`, `probe`,
  `sandbox`, `artifact`, or `quarantine`; it is a logical locator, never a path.
- `ArtifactRole`: `baselineDistribution`, `refreshDistribution`,
  `ordinaryResult`, or internal layer-bound `supportRecoveryDistribution`. The
  latter is recovery evidence only: no public create request selects it and no
  deploy/compare/supported-update input accepts it.
- `ArtifactKind`: `configurationDistribution`, `ordinaryConfiguration`,
  `configurationUpdate`, or `invalidArtifact`. This is probe/classification
  output, not a selectable workflow-input enum. `configurationUpdate` exists
  only so verification can classify/reject an out-of-scope CFU, and neither it
  nor `invalidArtifact` is accepted by any create/deploy/compare/update input.
- `AcceptedArtifactKind`: `configurationDistribution` or
  `ordinaryConfiguration`.
- `ConfigurationIdentity`: `{ metadataUuid: MetadataObjectId, name,
  vendor, version }`; `name` is 1-256 printable non-control Unicode scalars.
  `vendor` and `version` are each an explicit empty string or 1-256 printable
  non-control Unicode scalars; they are never omitted, `null`, or inferred.
- `TargetKind`: `task` or `original`; it selects a merge/apply destination and
  is distinct from `RepositoryTargetKind`.
- `OriginalInfobaseKind`: `file` or `clientServer`.
- `RepositoryTransport`: `file` or `server`.

Schema snapshots use these exact general bounds unless a field below is more
restrictive: names 1-256 characters, summaries 1-2048, task summaries/comments/
reasons/rationales 1-4096, display paths 1-4096, property paths 1-2048, and
redacted diagnostics 0-8192. Control characters are rejected except normalized
line feeds in narrative fields. General result arrays contain at most 1024
items; metadata object/property/reference collections contain at most 100000.
Every nested collection item is a named closed `$defs` record in the committed
schema snapshot. A collection with semantic invariants (canonical order,
uniqueness, non-empty membership, endpoint coverage, or reverse-release order)
is represented by a validated constructor/newtype whose deserializer enforces
those invariants. A length-bounded `BoundedVec` alone is not a valid public or
Serde construction path for such a collection. Task 6 creates the one shared
`RequiredNullable<T>` wrapper for a required key whose value is `T | null`.
Every physical containing field with those wire semantics carries a field-level
`#[serde(deserialize_with = "RequiredNullable::deserialize_required")]`; the
closed containing record requires the key, the wrapper serializes and
deserializes exactly `T` or JSON `null`, and neither `default` nor a
skip-serialization attribute is legal. An omitted key is not equivalent to
`null` and is rejected before domain construction. Tasks 7 and 8 reuse this
wrapper and deserializer rather than defining local nullable types or relying on
`Option<T>` behavior.

Task 6 also owns fail-closed Draft 2020-12 positional-array support in
`contracts/schema.rs`. Every exact tuple schema uses non-empty `prefixItems`,
`items: false`, and `minItems == maxItems == prefixItems.length`. The recursive
schema audit rejects an open tuple tail, a length mismatch, legacy array-valued
`items`, `additionalItems`, or any attempt to interpret the tuple under an older
dialect. Later tasks reuse this support for fixed action/decision tuples; they
do not emit a second tuple-schema dialect.

`SupportTransition` is a closed `oneOf` of
`enableConfigurationChanges { transitionKind: enableConfigurationChanges,
configurationDisplay, layerId, fromEnabled: false, toEnabled: true }`,
`restoreConfigurationChangesDisabled { transitionKind:
restoreConfigurationChangesDisabled, configurationDisplay, layerId,
fromEnabled: true, toEnabled: false }`, `makeObjectEditable { transitionKind:
makeObjectEditable, objectId, objectDisplay, layerId, fromState: locked,
toState: editable }`, or `restoreObjectLocked { transitionKind:
restoreObjectLocked, objectId, objectDisplay, layerId, fromState: editable,
toState: locked }`. `transitionKind` is the required discriminator; field-shape
guessing is forbidden. A transition list is ordered first by that declaration
order, then by `layerId` for configuration transitions or canonical
`MetadataObjectId` followed by `layerId` for object transitions. A duplicate
kind/semantic target, or contradictory forward/restore entries for one target
in the same semantic list, is rejected even when display text differs. Display
fields are bounded presentation data bound into the transition digest so status
can reproduce exact human instructions after a lost response.
`SupportBlocker` is closed `{ objectId, objectDisplay, layerId?, reason,
diagnostic }`, where `reason` is `configurationChangesDisabled`,
`objectLocked`, `vendorRestriction`, `offSupportRequired`,
`classificationIncomplete`, or `diagnosticCoverageIncomplete` and the bounded
diagnostic is redacted. `SupportPrerequisiteMismatchKind` is closed
`noAuthorizedVersionObserved`, `versionUnattributed`, `reservedAccountUsed`,
`multipleAuthorizedVersions`, `unauthorizedContentChanged`, `targetModeMismatch`,
`unexpectedSupportTransition`, `supportLayerChanged`, `offSupportObserved`,
`overlappingExternalSupportChange`,
`armingOrderViolated`,
`rootLockRetained`, `manualActorLockInventoryChanged`,
`reservedOriginalUsed`, or `originalNotClean`.
`SupportMissingEvidenceKind` is the closed enum
`candidateClassificationUnavailable`, `diagnosticCoverageIncomplete`,
`supportGraphIncomplete`, `recoverySourceMissing`,
`recoveryArtifactMissing`, `recoveryArtifactStale`,
`recoveryArtifactKindMismatch`, `configurationUpdateRejected`,
`recoveryCapabilityUnproven`, `recoveryLayerIdentityMismatch`,
`recoveryHandoffUnavailable`, `recoveryHandoffUnreadable`,
`recoveryRetentionLeaseBroken`,
`manualLeaseBusy`, `manualLeaseEffectUnknown`, `manualBaselineDirty`,
`manualBaselineInspectionUnproven`, `manualCapabilityUnproven`,
`repositoryActorUnavailable`, `manualTargetModeUnavailable`,
`workingInfobaseIdentityUnavailable`, `rootDeltaUnavailable`,
`contentDeltaUnavailable`, `ownershipEvidenceUnavailable`,
`supportLayerIdentityUnavailable`, or `repositoryHistoryCoverageIncomplete`.
Every Task 8 enum list/projection is duplicate-free and follows its enum's
declaration order unless the contract gives a literal tuple. This applies in
particular to `SupportPrerequisiteMismatchKind[]`,
`SupportMissingEvidenceKind[]`, `SupportCandidateReason[]`,
`SupportArmStaleKind[]`, and `VendorSupportDecision[]`; localized labels or
lexical enum spelling never choose order.
`SupportEvidenceGap` is a closed tagged `oneOf` of:

- `candidateEvidence { gapKind: candidateEvidence, objectId, objectDisplay,
  layerId?, missingEvidenceKind: candidateClassificationUnavailable |
  diagnosticCoverageIncomplete, diagnostic }`;
- `supportLayerRecoveryEvidence { gapKind: supportLayerRecoveryEvidence,
  layerId, missingEvidenceKind: recoverySourceMissing |
  recoveryArtifactMissing | recoveryArtifactStale |
  recoveryArtifactKindMismatch | configurationUpdateRejected |
  recoveryCapabilityUnproven | recoveryLayerIdentityMismatch |
  recoveryHandoffUnavailable | recoveryHandoffUnreadable |
  recoveryRetentionLeaseBroken, diagnostic }`;
- `unidentifiedSupportLayerEvidence { gapKind:
  unidentifiedSupportLayerEvidence, layerObservationDigest,
  missingEvidenceKind: supportLayerIdentityUnavailable, diagnostic }`;
- `manualWorkingInfobaseEvidence { gapKind: manualWorkingInfobaseEvidence,
  workingInfobaseIdentity, missingEvidenceKind: manualLeaseBusy |
  manualLeaseEffectUnknown | manualBaselineDirty |
  manualBaselineInspectionUnproven | manualCapabilityUnproven,
  diagnostic }`; or
- `prerequisiteVersionEvidence { gapKind: prerequisiteVersionEvidence,
  supportActionId, repositoryVersion: RepositoryVersion | null,
  missingEvidenceKind: repositoryActorUnavailable |
  manualTargetModeUnavailable | workingInfobaseIdentityUnavailable |
  rootDeltaUnavailable | contentDeltaUnavailable |
  ownershipEvidenceUnavailable | supportLayerIdentityUnavailable |
  repositoryHistoryCoverageIncomplete, diagnostic }`; or
- `repositoryHistoryEvidence { gapKind: repositoryHistoryEvidence,
  fromCursor: RepositoryHistoryCursor, firstObservedVersion?:
  RepositoryVersion, missingEvidenceKind: repositoryActorUnavailable |
  rootDeltaUnavailable | contentDeltaUnavailable |
  ownershipEvidenceUnavailable | supportLayerIdentityUnavailable |
  supportGraphIncomplete | diagnosticCoverageIncomplete |
  repositoryHistoryCoverageIncomplete, diagnostic }`; or
- `globalSupportEvidence { gapKind: globalSupportEvidence,
  missingEvidenceKind: supportGraphIncomplete |
  diagnosticCoverageIncomplete, diagnostic }`.

The enum values come from the bounded diagnostic capability registry; they are
not localized-message text. This union represents evidence gaps that do not
truthfully identify a blocked candidate,
so no fake object ID is generated for a missing layer distribution, working-IB
baseline, lease, repository successor, or global support-graph proof. In
`repositoryHistoryEvidence`, `firstObservedVersion` is present exactly when the
immediate successor is capability-proven and absent when the gap is
`repositoryHistoryCoverageIncomplete`.
Task 8 canonical collections use typed semantic keys, never presentation text.
`SupportBlocker[]` is ordered by canonical `objectId`, nullable `layerId`
(`null` first), then `reason` declaration order; duplicate semantic keys are
rejected. `SupportEvidenceGap[]` is ordered by `gapKind` declaration order,
then its typed identity (`objectId`/nullable layer, layer ID, layer-observation
digest, working-IB identity digest, support-action/version, or history cursor),
then `missingEvidenceKind` declaration order. Every nullable identity component
sorts `null` before a typed value. Repository versions and cursors in those keys
use capability-proven history order; their opaque strings are never lexically or
numerically ordered. The global variant has no fabricated identity.
Duplicate gap keys are rejected even when diagnostics differ.
`SupportRootLockObservation` is closed `{ mode: readOnlySnapshot,
completeness: readOnlySnapshotProven, owner: RepositoryOwnerIdentity | null,
observationDigest }`. `SupportRootLockObservationDigestRecord` is the closed
`{ mode, completeness, owner }`, retaining explicit owner `null`, and
`observationDigest ==
sha256(canonical(SupportRootLockObservationDigestRecord))`; it is optional
preview diagnostics and never closes a race. `SupportRootLockProof` is closed `{ mode:
acquireRecheckReleaseGuard, guardReceiptId: UnicaId,
rootGuardReleaseReceiptId: UnicaId,
acquiredByReservedAccount: true,
historyRecheckedUnderGuard: true, supportGraphRecheckedUnderGuard: true,
originalRecheckedUnderGuard: true, releaseVerified: true,
authorizationOutcome: consumed | cancelled | unchanged,
reservedOriginalTerminalizationProofDigest?: Sha256, observationDigest }`.
`SupportRootLockProofDigestRecord` is the same closed record with only top-level
`observationDigest` removed; `observationDigest ==
sha256(canonical(SupportRootLockProofDigestRecord))`.
Every support reconciliation/cancellation apply must
journal and acquire the root, repeat all approved checks while holding it,
and verify release. Completed reconciliation requires `consumed`; completed
cancellation requires `cancelled`. A stopped guarded inspection may use
`unchanged` only with explicit no-effect data. No read-only snapshot alone is
terminal proof. The proof record itself requires the terminalization digest to
be absent for `authorizationOutcome: unchanged`. For `consumed`/`cancelled`, its
required-versus-absent rule cannot be decided by this nested record because it
has no `manualTargetMode`: the outer completed or stopped result constructor
validates it against the enclosing authorization. It is required exactly for
`reservedOriginal`, absent for `separateWorkingInfobase`, and when present
equals the enclosing `ReservedOriginalTerminalizationProof.proofDigest`; no
nested mode is inferred.
`rootGuardReleaseReceiptId` is the verified release receipt for the same
acquisition identified by `guardReceiptId`; both are covered by
`observationDigest` and cannot be spliced across attempts.
`SupportActionArmingReceipt` is closed `{ armingReceiptId, supportActionId,
supportActionDigest, expectedBeforeHistoryCursor: RepositoryHistoryCursor,
armingCursor: RepositoryHistoryCursor, historyPartition:
RepositoryHistoryPartition, supportGateDigest, candidateSetDigest,
expectedRelevantBaselineDigest, supportGraphDigest,
supportRecoveryDistributionSetDigest, originalFingerprint,
manualTargetMode, rootLockObservation: SupportRootLockObservation,
rootHeldByManualActor: true,
authorizedVersionMustBeFirstRootSupportAfterCursor: true, receiptDigest }`.
`SupportActionArmingReceiptDigestRecord` is the same closed record with only
top-level `receiptDigest` removed; `receiptDigest ==
sha256(canonical(SupportActionArmingReceiptDigestRecord))`.
Its partition starts exactly at `expectedBeforeHistoryCursor`, ends at
`armingCursor`, and contains only `unrelatedRoutine` entries. The root
observation has the bound manual actor as owner. The receipt is immutable and
is carried by every later classification, reconciliation, cancellation, frozen
recovery, and archive record for that action.
`SupportArmStaleEvidence` is closed `{ expectedBeforeHistoryCursor,
observedHistoryCursor, historyPartition: RepositoryHistoryPartition,
mismatchKinds: SupportArmStaleKind[], expectedSupportGateDigest,
observedSupportGateDigest, expectedRelevantBaselineDigest,
observedRelevantBaselineDigest, expectedSupportGraphDigest,
observedSupportGraphDigest, expectedRecoveryDistributionSetDigest,
observedRecoveryDistributionSetDigest, expectedOriginalFingerprint,
observedOriginalFingerprint, observedRootLock: SupportRootLockObservation,
evidenceDigest }`, where `SupportArmStaleKind` is
the closed enum `historyChanged`, `supportGateChanged`,
`relevantBaselineChanged`, `supportGraphChanged`,
`recoveryDistributionSetChanged`, or `originalFingerprintChanged`. The
non-empty canonical mismatch list is the union of the paired digest/fingerprint
inequalities plus `historyChanged` exactly when the complete partition contains
an entry other than `unrelatedRoutine`. `relevantBaselineChanged` is also
present when that partition contains any relevant/support/invalid/corrective
entry even if a later revert makes the net relevant-baseline digest equal.
An all-`unrelatedRoutine` cursor advance is a valid arming prefix and is never a
stale mismatch by itself. The history partition starts at the authorization
cursor, ends exactly at `observedHistoryCursor`, and is the complete contiguous
range between those endpoints; a net-revert tail cannot be omitted.
`SupportArmStaleEvidenceDigestRecord` is the same closed record with only
top-level `evidenceDigest` removed; `evidenceDigest ==
sha256(canonical(SupportArmStaleEvidenceDigestRecord))`. In
`SupportArmStaleData`, `requiredExternalAction` is present exactly when this
observation names the bound manual actor as current root owner; its owner,
repository username, target mode, and conditional working-IB identity are
reproduced byte-for-byte. Otherwise the field is absent, and the later explicit
cancellation performs its own guarded ownership checks.
`ManualActorLockInventoryProof` is closed `{ username,
completeness: readOnlySnapshotProven, baselineLockSetDigest,
observedLockSetDigest, unchangedFromBaseline: true, rootAbsent: true,
baselineWasEmpty: true, observationDigest }`.
`ManualActorLockInventoryProofDigestRecord` is the same closed record with only
top-level `observationDigest` removed; `observationDigest ==
sha256(canonical(ManualActorLockInventoryProofDigestRecord))`. It is required
only for
`reservedOriginal`: the authorization binds the reserved actor's proven empty
baseline before the human window and the observed set must return to that empty
baseline after root release, so a same-user candidate lock cannot hide behind a
free root. A topology without complete actor-lock inventory cannot enable
`reservedOriginal`. In `separateWorkingInfobase` this proof and baseline are
absent; non-root locks owned by the distinct human actor remain ordinary foreign
locks for later planning/acquisition, while root release is still mandatory.
`ReservedOriginalLeaseStopEvidence` is closed `{ cause:
designerSessionOpenOrLeaseBusy, reservedOriginalIdentityDigest,
exclusiveLeaseCapabilityId, leaseOwner: RepositoryOwnerIdentity | null,
exclusiveLeaseAcquired: false, evidenceDigest }`.
`ReservedOriginalLeaseStopEvidenceDigestRecord` is the same closed record with
only top-level `evidenceDigest` removed; `evidenceDigest ==
sha256(canonical(ReservedOriginalLeaseStopEvidenceDigestRecord))`. It is legal
only for a
capability-proven clean lease rejection before inspection; an unknown acquire,
inspection, or release outcome is never encoded as this retryable stop.

`SupportCandidateReason` is the closed declaration-order enum
`platformComparison`, `canonicalDelta`, `ownership`, `addDelete`, or
`referenceClosure`. `SupportCandidate` is closed `{ objectId, objectDisplay,
layerId?, repositoryAction, currentState, vendorRestriction, requiredState,
reasons: SupportCandidateReason[] }`.
`repositoryAction` is `add`, `modify`, or `delete`; `currentState` is
`notApplicable`, `locked`, `editable`, or observed `offSupport`, while
`requiredState` is `notApplicable`, `editable`, `preserveOffSupport`, or
`offSupportRequired`. `preserveOffSupport` is legal only with
`currentState: offSupport`, creates no support transition, and requires the
sandbox/result graph to preserve that exact pre-existing mode.
`offSupportRequired` is evidence only, can never be supplied to
`unica.support.edit`, and makes the candidate a `vendorForbidsChanges` blocker.
`layerId` is absent exactly when both states and `vendorRestriction` are
`notApplicable`; otherwise it is required. `vendorRestriction: unknown`
requires an inconclusive blocker and cannot appear in a more permissive
outcome.
`reasons` is non-empty, duplicate-free, and ordered exactly by
`SupportCandidateReason` declaration order; every literal requires its matching
typed producer evidence/digest and is never inferred from free-form diagnostics.
A candidate list is ordered by canonical `objectId` then nullable `layerId`
(`null` first) and contains exactly one record per semantic identity; duplicates
with equal or differing state/action/reasons are rejected. The closed
`SupportCandidateSetDigestRecord` is `{ candidateSetId: UnicaId, candidates:
SupportCandidate[] }`, and `candidateSetDigest ==
sha256(canonical(SupportCandidateSetDigestRecord))`.
`SupportRecoveryDistributionHandoff` is closed `{ handoffId: UnicaId,
profileArtifactRefId: ProfileArtifactRefId, profileArtifactDisplay,
userVisibleFileName,
manualTargetMode: ManualSupportTargetMode, manualActorUsername,
workingInfobaseIdentity?: ManualWorkingInfobaseIdentity, layerId,
distributionArtifactId, artifactSha256, readabilityProbeReceiptId: UnicaId,
manualReadabilityCapabilityRowId: CapabilityRowId,
retentionLeaseId: UnicaId, retentionReceiptId: UnicaId,
retentionCapabilityRowId: CapabilityRowId,
retentionOwner: externalProfile,
retentionPolicy: profileManagedAtLeastUntilTaskArchive,
retentionLeaseHeld: true, contentMutationRejectedWhileHeld: true,
availableToManualActor: true }`.
`retentionCapabilityRowId` resolves only through the tracked closed manifest
`plugins/unica/references/branched-development/retention-provider-capabilities.json`,
not through an arbitrary profile string. A `RetentionProviderCapabilityRow` is
closed `{ id: CapabilityRowId, schemaVersion, featureContractVersion,
contractDigest, host { os, arch }, providerKind, providerVersion,
storageKind, storageVersion, harnessDigest, implementationCommit, passedAt,
cases: RetentionProviderCapabilityCaseEvidence[], evidence { path, sha256 },
passed: true }`, where each closed case is `{ caseId, resultDigest,
postconditionDigest }`, case IDs are unique, and the list is canonical.
Phase 1 Task 8 carries and validates only typed `CapabilityRowId` references in
support records. Parsing and validating this manifest row and its case set stays
owned by roadmap Phase 3; Task 8 must not add a second row parser or accept a
profile string in place of the typed ID.
The required exact case set proves idempotent acquire/replay, exact held-state
observation, manual-actor readability of the bound SHA, rename/overwrite/delete
denial while held, exact-once release/replay, unknown-effect reconciliation,
and canonical path/symlink/reparse containment. Evidence is tracked, redacted,
machine-readable, and its SHA plus contract/harness digests are recomputed by
CI. `branched.start` rejects a missing/stale row, provider/host/storage mismatch,
skipped case, digest mismatch, or unpassed evidence before task creation.
`manualReadabilityCapabilityRowId` separately names the exact platform
capability row whose required handoff-readability case proves that Designer for
the bound actor can open the same ordinary CF/SHA; the two capability IDs are
never conflated.
The working-IB field follows the target-mode presence rule. The reference names
a pre-existing profile-managed, user-visible immutable CF source; the display
is bounded presentation text and the 1-255-character leaf ends in `.cf` and
contains no slash, backslash, control character, `.`/`..` identity, absolute
path, credential, or arbitrary
destination. A real capability fixture proves that the bound actor can open the
same SHA-256 CF in Designer and that the profile owner retains it at least until
task archive through a capability-proven WORM/retention lease that rejects
overwrite/delete while held. Unica only probes/imports it into owned evidence storage: it never
writes, overwrites, quarantines, or deletes the external source. Cleanup drops
only the task reference. Thus the contained preflight policy and owned-root
deletion rules remain intact, while off-support recovery does not depend on an
opaque server-only artifact ID.

`SupportRecoveryHandoffRevalidation` is closed `{ handoffId: UnicaId,
retentionLeaseId: UnicaId, expectedArtifactSha256, observedArtifactSha256,
retentionLeaseStillHeld: true, readableByManualActor: true,
revalidationReceiptId: UnicaId,
manualReadabilityCapabilityRowId: CapabilityRowId,
retentionCapabilityRowId: CapabilityRowId,
revalidationDigest }`; both SHA fields are equal and the IDs/capability equal the
frozen handoff: each capability field equals its same-named handoff field and
both are covered by `revalidationDigest`. It is refreshed immediately before a corrective instruction is
published. A missing/replaced/unreadable source or broken retention lease is a
capability breach and no human correction instruction is issued.

`SupportRecoveryDistributionEvidence` is closed `{ layerId,
distributionArtifactId, role: supportRecoveryDistribution,
verifiedKind: configurationDistribution,
artifactSha256, vendorLayerIdentityDigest, capabilityRowId,
handoff: SupportRecoveryDistributionHandoff, evidenceDigest }`.
Before publishing any manual support authorization, Unica persists one verified
ordinary vendor distribution for every support layer reachable from the
configuration-root support-settings window, not merely candidate layers, and proves the
recovery-only restoration capability. A CFU is outside this workflow and
cannot satisfy this evidence. Missing or stale evidence downgrades an
otherwise-manual outcome to `supportPreflightInconclusive`; it is not evaluated
to mask an already exact `ready` or `vendorForbidsChanges` outcome. Recovery
therefore never discovers too late that an `offSupport` mistake is impossible
to restore.
Each `evidenceDigest == sha256(canonical(evidence-without-evidenceDigest))` and
`supportRecoveryDistributionSetDigest ==
sha256(canonical(layerId-sorted-evidence-digests))`.
Main preflight obtains these through an internal contained resolver that imports
a profile-registered immutable vendor artifact, then registers/verifies it with
role `supportRecoveryDistribution`, matching layer identity and SHA-256, and
probes the bound user-visible handoff read-only. A platform-held layer without
that retained profile source is insufficient for manual authorization and is
inconclusive. Handoff layer/artifact/SHA fields equal the
enclosing evidence byte-for-byte. The source
path/credential never crosses MCP, arbitrary caller artifacts are forbidden,
and a missing capability/source is inconclusive. This role is explicitly
rejected by `delivery.deploy`, `merge.compare`, and
`merge.prepare(supportedUpdate)`.
`SupportPreflightData` is closed
`{ supportGateId, outcome, candidateSetId, candidateSetDigest, gateInputs:
SupportGateInputDigests, candidates:
SupportCandidate[], blockers: SupportBlocker[], evidenceGaps:
SupportEvidenceGap[], supportGraphDigest,
requiredTransitions: SupportTransition[], surplusTransitions:
SupportTransition[], observedHistoryCursor: RepositoryHistoryCursor,
relevantBaselineDigest: RelevantBaselineDigest, originalFingerprint,
ordinaryResultArtifactId, comparisonId, settingsDigest, sandboxResultDigest,
supportRecoveryDistributions: SupportRecoveryDistributionEvidence[],
supportRecoveryDistributionSetDigest,
capabilityRowId, supportGateDigest, historyEvidence:
SupportGateHistoryEvidence, supportActionId?, supportActionDigest? }`.
The action fields are required together only for `manualSupportRequired` and
absent for all other outcomes. Each blocker repeats an exact candidate
identity using `SupportBlocker`. Outcome selection is staged and deterministic:
incomplete candidate/global support classification is inconclusive first; an
exact vendor prohibition is next; only an otherwise-manual transition set then
loads/probes recovery distributions and downgrades to inconclusive if that
manual-safety evidence is incomplete; complete manual and ready are last. Thus
a recovery-source gap cannot mask exact ready/vendor classification, while a
mixed or incomplete candidate classification cannot report `ready`. All
outcomes except `supportPreflightInconclusive` require an empty evidence-gap
list. `ready` requires
empty blockers/required/surplus transitions; `manualSupportRequired` requires a
  non-empty required transition set, with `surplusTransitions` an exact subset of
  its restore transitions, plus complete recovery-distribution evidence for
  every support layer reachable from the configuration-root support-settings
  window. `vendorForbidsChanges` requires non-empty typed candidate blockers.
  `supportPreflightInconclusive` requires a non-empty canonical union of
  candidate-shaped incomplete blockers and/or `evidenceGaps`, and no action
  authorization.
The two recovery-distribution fields remain schema-required for every outcome.
For `ready` and `vendorForbidsChanges` they are respectively the empty list and
`sha256(canonical([]))`. For `manualSupportRequired` they are the complete
root-reachable layer set and its canonical digest. For an otherwise-manual
`supportPreflightInconclusive`, the list contains exactly the successfully
proven canonical subset and the gaps identify every missing/stale remainder;
for candidate/global inconclusive classification before a provisional manual
result, the list/digest are empty. No other cardinality is legal.
An exact `offSupport -> preserveOffSupport` candidate may participate in
`ready` only when the task operation needs no new detachment and the no-force
sandbox proves graph preservation. A newly required detachment remains
`offSupportRequired` and therefore `vendorForbidsChanges`.
`requiredTransitions` is the full authorized list, including every surplus
restore entry; in a surplus-only case it equals non-empty
`surplusTransitions` rather than becoming empty.
`gateInputs.candidateSetDigest`, `supportGraphDigest`,
`supportRecoveryDistributionSetDigest`, `settingsDigest`, and the digests of the
named canonical delta, ordinary-result artifact, sandbox result, capability row,
and original fingerprint equal their same semantic sources byte-for-byte. The
closed `SupportGateDigestRecord` is `{ supportGateId, outcome, candidateSetId,
gateInputs, relevantBaselineDigest, ordinaryResultArtifactId, comparisonId,
capabilityRowId, blockers, evidenceGaps, requiredTransitions,
surplusTransitions }`, and `supportGateDigest ==
sha256(canonical(SupportGateDigestRecord))`. It deliberately excludes
`observedHistoryCursor`, replaceable `historyEvidence`, the digest field itself,
and both support-action projection fields. Thus replacing only revalidated
all-unrelated history evidence cannot create a digest cycle or mutate the
semantic gate identity.
At publication, `historyEvidence` has equal gate/classified cursors and an empty
partition. A later consumer may atomically replace only that evidence with a
longer all-`unrelatedRoutine` partition after revalidation; semantic gate inputs
and `supportGateDigest` remain unchanged. The current evidence digest is a
required downstream CAS input.

The two action fields in `SupportPreflightData` are only a sibling projection;
there is no nested authorization inside this record. Their byte-for-byte
equality with the outer stopped result's `SupportActionAuthorizationData` is an
outer `SupportPreflightStopData` invariant implemented with Task 15. No bound
field may be omitted or replaced after a lost response, and the gate record's
canonical blocker/evidence-gap lists prevent replay with fabricated or omitted
missing evidence.
For separate mode, publication first acquires the service-held exclusive lease,
proves the working IB current equals its recorded repository base, persists the
full `ManualWorkingInfobaseBaseline`, and verifies lease release. Its identity
and lease-capability ID equal the profile-bound authorization fields; its root
target/support graph equal the current gate root/support state at
`historyEvidence.classifiedThroughCursor`. A recorded object map may lag only
by completely classified unrelated non-root versions and never by a root or
support change. Busy, dirty,
or unknown baseline inspection yields `supportPreflightInconclusive` and no
authorization/instruction.
At authorization publication, `expectedBeforeHistoryCursor` equals both
`SupportPreflightData.observedHistoryCursor` and
`historyEvidence.classifiedThroughCursor`; no caller or adapter may choose a
shorter later range.

`SupportActionAuthorizationData` is closed `{ supportActionId, purpose,
supportActionDigest, supportGateId, supportGateDigest, candidateSetDigest,
expectedBeforeHistoryCursor: RepositoryHistoryCursor,
expectedRelevantBaselineDigest: RelevantBaselineDigest,
armingRequired: true,
authorizedTransitions: SupportTransition[],
authorizedTransitionsDigest,
supportRecoveryDistributions: SupportRecoveryDistributionEvidence[],
supportRecoveryDistributionSetDigest,
manualTargetMode: ManualSupportTargetMode, reservedIntegrationUsername,
reservedOriginalIdentityDigest, reservedOriginalLeaseCapabilityId?,
expectedOriginalFingerprint,
manualActorUsername, manualActorLockBaselineDigest?,
manualWorkingInfobaseIdentity?: ManualWorkingInfobaseIdentity,
manualWorkingInfobaseBaseline?: ManualWorkingInfobaseBaseline,
armingReceipt?: SupportActionArmingReceipt,
originPhase, cancelledPhase, relevantAdvancePhase, postReconcilePhase,
phaseEvidenceDigest, state, freezeKind? }`, where `purpose` is
`mainIntegrationPrerequisite` or `abandonmentCleanup` and `state` is
`awaitingArm`, `armed`, `consumed`, `cancelled`, or `frozenForRecovery`.
`authorizedTransitionsDigest == sha256(canonical(authorizedTransitions))` over
the canonical transition list. `SupportActionDigestRecord` is the closed `{
supportActionId, purpose, supportGateId, supportGateDigest, candidateSetDigest,
expectedBeforeHistoryCursor, expectedRelevantBaselineDigest, armingRequired,
authorizedTransitions, authorizedTransitionsDigest,
supportRecoveryDistributions, supportRecoveryDistributionSetDigest,
manualTargetMode, reservedIntegrationUsername, reservedOriginalIdentityDigest,
reservedOriginalLeaseCapabilityId?, expectedOriginalFingerprint,
manualActorUsername, manualActorLockBaselineDigest?,
manualWorkingInfobaseIdentity?, manualWorkingInfobaseBaseline?, originPhase,
cancelledPhase, relevantAdvancePhase, postReconcilePhase,
phaseEvidenceDigest }` with the same exact purpose/mode presence rules as the
authorization. `supportActionDigest ==
sha256(canonical(SupportActionDigestRecord))`. This acyclic record is precisely
the immutable authorization projection: it excludes `supportActionDigest`
itself and the mutable/terminal `armingReceipt`, `state`, and `freezeKind`
members. Those fields may evolve only under the bound digest and can never feed
back into it.
Publication creates `awaitingArm` with no arming receipt. Only the exact
`repository.update(mode="supportPrerequisiteArm")` apply may change it to
`armed` and attach the immutable receipt. The receipt is required for `armed`,
`consumed`, and for `cancelled` iff cancellation followed arming. `freezeKind`
is the closed enum `armedAction | preArmCancellationEffect` and is required
exactly for `frozenForRecovery`. A frozen `armedAction` requires the immutable
arming receipt; a frozen `preArmCancellationEffect` requires it to be absent,
because the action was never armed. The receipt and `freezeKind` are both absent
for `awaitingArm`, and `freezeKind` remains absent for every terminal state.
Thus an unknown pre-arm cancellation effect can freeze further mutation without
retroactively granting edit authority.
Pre-arm non-unrelated/relevant-support history, or bound support/original/
handoff drift, makes this authorization unarmable and requires fresh preflight
rather than freezing an action that was never allowed to edit. An all-unrelated
contiguous history prefix is explicitly admissible. Both read-only preview and apply-final-recheck staleness
leave it `awaitingArm`; only the exact explicit cancellation flow may
terminalize it after the human root window is proven closed.
An awaiting/armed authorization is not readiness evidence. It may survive
global history-cursor advances only for typed arming,
reconciliation/cancellation. Reconciliation is legal only from `armed`; its
authorized version must be the first root/support version strictly after the
receipt's `armingCursor`, and exactly one version may contain the authorized
support transitions. The prefix from `expectedBeforeHistoryCursor` through the
arming cursor is all-unrelated; other later intervening versions must be
semantically partitioned routine or proven disjoint external-support changes.
The action may be consumed solely by
`repository.update(mode="supportPrerequisite")`. While it is awaiting/armed,
task mutation, abandonment, candidate/transition replacement, and
other authoritative task effects are rejected with
`supportPrerequisiteReconciliationRequired`. The caller must first use the
typed `repository.update(mode="supportPrerequisiteCancellation")` preview/apply;
only that flow may prove no matching external version, original/support change,
or root lock and atomically cancel the authorization. Stage is decisive. From
`armed`, a matching attributable version still requires normal prerequisite
reconciliation. From `awaitingArm`, any complete root/support version is
classified `preArmExternal` even when actor/IB/delta equals the proposal: the
cancellation flow preserves that external baseline, cancels with arming fields
absent, selects `relevantAdvancePhase`, and archives the ordering violation.
Incomplete pre-arm attribution returns the typed cancellation inconclusive stop
and remains awaiting until evidence is complete; it never retro-arms or enters
receipt-required frozen recovery. Only an unknown effect after cancellation
intent may freeze it as `preArmCancellationEffect`, with no arming receipt and
the exact two-stage observation/reapproval recovery defined below.

For `mainIntegrationPrerequisite`, `originPhase`/`cancelledPhase` are
`synchronized`, and both advance/reconcile phases are `localVerified`. For
`abandonmentCleanup`, `postReconcilePhase` is `abandonmentReady`,
`cancelledPhase` is the exact safe origin, and `relevantAdvancePhase` is the
precomputed safe ancestor justified by the task's current checkpoint (never an
upgrade). These four fields and their source checkpoint/evidence digest are
covered by `supportActionDigest`; no repository result chooses a more permissive
phase after the fact.

In `reservedOriginal` mode the authorization creates a bounded, journaled human
window in which that exact original and reserved repository username may be
used only for root lock, authorized support transitions, one root commit, and
release. The working-infobase/baseline fields are absent,
`reservedOriginalLeaseCapabilityId` is required, `manualActorUsername` equals
the reserved username, and the bound actor-lock baseline is required and empty.
The lease capability equals the validated `StartData`/profile capability and
cannot change for the action.
Reconciliation must prove the
original moved from `expectedOriginalFingerprint` to a clean repository-equal
root-only result. In `separateWorkingInfobase` mode the working-infobase and
matching clean-baseline fields are required and both reserved-only capability/
lock-baseline fields are absent; they bind the exact
profile-declared working infobase, and the manual actor
username differs from the reserved username, capability-proven history
identifies the bound different infobase, and the reserved original retains its
expected fingerprint until Unica updates it. Any cross-mode observation is
`targetModeMismatch`; absent proof is `supportPreflightInconclusive`, not
inference.
The baseline's `workingInfobaseIdentity` equals
`manualWorkingInfobaseIdentity`, its `exclusiveLeaseCapabilityId` equals the
profile inspection capability, and its `supportGraphDigest` equals the gate's
current support graph; none may be substituted by another clean IB.

`AcquireSupportRootInstruction` is closed `{ kind: acquireSupportRoot,
supportActionId, purpose, manualTargetMode, repositoryUsername,
workingInfobaseIdentity?: ManualWorkingInfobaseIdentity, lockTarget:
configurationRoot, lockCandidateObjects: false, doNotEditBeforeArmed: true,
doNotCommitBeforeArmed: true, resumeWith: repository.update,
resumeMode: supportPrerequisiteArm }`. It is the only instruction published by
the initial `manualSupportRequired` stop; the working-IB field is required only
in separate mode.
`ManualSupportInstruction` is closed `{ kind: manualSupportAction,
supportActionId, purpose, armingReceiptId, armingReceiptDigest,
armingCursor: RepositoryHistoryCursor, manualTargetMode, repositoryUsername,
workingInfobaseIdentity?: ManualWorkingInfobaseIdentity, rootAlreadyLocked:
true, requestedRootLockProcedure: retainThroughCommit,
lockCandidateObjects: false,
authorizedVersionMustBeFirstRootSupportAfterCursor: true, transitions:
SupportTransition[], commitAsSeparateRootVersion: true, releaseRoot: true,
closeReservedOriginalDesignerSession?: true,
closeWorkingInfobaseSession?: true, resumeWith: branched.status }`. It is
published only by a completed `supportPrerequisiteArm`; all arming fields equal
its receipt. Exactly one close-session field is present: reserved-original for
`reservedOriginal`, working-IB for `separateWorkingInfobase`. Every other field
is reconstructed from the authorization/arming receipt after response loss.
The requested procedure tells the human to retain the already-held root, but
the platform contract does not claim a capability it cannot observe
continuously. Terminal acceptance instead proves the exact actor/infobase and
authorized delta, with the accepted version as the first root/support version
after `armingCursor`. Releasing and reacquiring the root is therefore not by
itself a mismatch when no intervening root/support version exists; such an
intervening version remains disqualifying regardless of a later net revert.
`VendorSupportDecision` is the closed declaration-order enum
`changeTaskScope`, `useNewerVendorDelivery`, or `safeAbandonment`; the three
literals map one-to-one to the ADR's only exits from `vendorForbidsChanges`.
`VendorSupportDecisionInstruction` is closed `{ kind: vendorSupportDecision,
blockers: SupportBlocker[], allowedDecisions: [changeTaskScope,
useNewerVendorDelivery, safeAbandonment] }`. No fourth/free-form decision or
different order is valid. `SupportEvidenceInstruction` is closed `{ kind:
provideSupportEvidence, blockers: SupportBlocker[], evidenceGaps:
SupportEvidenceGap[], missingEvidenceKinds: SupportMissingEvidenceKind[],
resumeWith: branched.status, supportEvidenceInstructionDigest }`. Its
`SupportEvidenceInstructionDigestRecord` is the same closed record with only
top-level `supportEvidenceInstructionDigest` removed, and
`supportEvidenceInstructionDigest ==
sha256(canonical(SupportEvidenceInstructionDigestRecord))`. Its blockers/gaps reproduce the exact stopped
inspection evidence, and `missingEvidenceKinds` is their canonical deduplicated
projection in `SupportMissingEvidenceKind` declaration order. For an inconclusive support preflight, both lists equal the nested
preflight lists byte-for-byte.
`ReleaseRepositoryLocksInstruction` is closed `{ kind:
releaseRepositoryLocks, owner: RepositoryOwnerIdentity | null,
objectDisplays: RepositoryTargetDisplay[], coordinationRequired: true,
resumeWith: branched.status, lockInstructionDigest }`.
`objectDisplays` is Unicode-scalar ordered and duplicate-free presentation data,
never lock identity. `ReleaseRepositoryLocksInstructionDigestRecord` is the same
closed record with only top-level `lockInstructionDigest` removed, and
`lockInstructionDigest ==
sha256(canonical(ReleaseRepositoryLocksInstructionDigestRecord))`.
An unknown owner remains explicit `null`; no username is inferred.
`CleanManualWorkingInfobaseInstruction` is closed `{ kind:
cleanManualWorkingInfobase, workingInfobaseIdentity,
closurePlanDigest, exclusiveLeaseCapabilityId,
expectedRepositoryFingerprint, reason: leaseBusy | localChanges,
closeDesignerSession: true, resumeWith: branched.status }`. It asks the
human to discard/reconcile local edits and close the Designer session; Unica
never performs that cleanup against an external human IB.
`CloseReservedOriginalDesignerInstruction` is closed `{ kind:
closeReservedOriginalDesigner, reservedOriginalIdentityDigest,
exclusiveLeaseCapabilityId, closeDesignerSession: true, resumeWith:
branched.status }`; it is emitted only for the capability-proven busy lease stop
and never substitutes for the terminal lease proof. None contains a
credential, command, or arbitrary path.
`SupportTransitionConflict` is closed `{ repositoryVersion,
repositoryActor: RepositoryActorIdentity | null, objectId?,
objectDisplay, layerId, authorizedTransition: SupportTransition,
externalTransitionDigest, overlapKind: sameTarget | layerDependency | unknown,
diagnostic }`. `objectId` is absent for configuration-level transitions and
required/equal to `authorizedTransition.objectId` for object-level transitions;
in both cases display/layer identity equals the authorized transition target.
A conflict list is ordered by capability-proven repository history position,
then the canonical `authorizedTransition` key, `externalTransitionDigest`, and
`overlapKind` declaration order. It never lexically orders opaque
`RepositoryVersion`; duplicate semantic conflict keys are rejected even when
diagnostics differ.
`SupportConflictInstruction` is closed `{ kind:
coordinateExternalSupportChange, conflictResolutionId, conflicts:
SupportTransitionConflict[], allowedEvidenceKinds:
[externalCorrectiveVersion, externalSupportOwnershipReceipt],
requiredFinalBaselineDigest, automaticReversalForbidden: true, resumeWith:
branched.status, supportConflictInstructionDigest }`;
`supportConflictInstructionDigest ==
sha256(canonical(instruction-without-supportConflictInstructionDigest))`. It
contains no acceptance boolean and cannot authorize Unica
to overwrite another actor. A corrective sequence or another task's immutable
support receipt must prove the final external baseline; a user assertion is not
evidence.

`ExternalSupportOwnershipEvidence` is a closed `oneOf` of `{ kind:
supportPrerequisiteReceipt, receiptId, receiptDigest }` or `{ kind:
capabilityProvenHistoryAttribution, repositoryActor,
attributionEvidenceDigest, capabilityRowId }`. The independent attribution
digest covers the raw history actor, root/content delta digests, and capability
receipt only; it is not the enclosing partition semantic digest and therefore
creates no digest cycle. This is platform/durable-state evidence, never a
caller assertion.

`SupportPrerequisiteVersionObservation` is a closed tagged `oneOf`. Every
variant has `{ repositoryVersion, classification, classificationDigest,
mismatchKinds[] }`. A collection of observations is unique by
`repositoryVersion` and ordered only by capability-proven repository-history
position, never by the opaque version string. Each `mismatchKinds` list is the
exact duplicate-free declaration-order projection. `authorized` additionally requires `repositoryActor`,
`supportActionId`, `supportActionDigest`, `armingReceiptId`,
`armingReceiptDigest`, `firstRootSupportAfterArming: true`,
`actionAttributionEvidenceDigest`,
`authorizedTransitionsDigest`, `manualTargetMode`,
`workingInfobaseIdentity?`, `rootDeltaDigest`,
`contentDeltaDigest: CanonicalEmptyDeltaDigest`,
`observedSupportTransitionsDigest`, and
`rootDeltaContainsOnlyAuthorizedSupportTransitions: true`; `routine` requires
`repositoryActor`, `relevance: unrelated | relevant`, both delta digests,
`supportTransitionsDigest: CanonicalEmptyDeltaDigest`,
`supportGraphUnchanged: true`, and forbids manual-action fields;
`externalSupport` requires the actor, both delta digests,
`provenNotThisAction: true`, `overlapWithAuthorizedTransitions: false`, and
`supportOnlyDelta: true`, `contentDeltaDigest: CanonicalEmptyDeltaDigest`,
`externalSupportDisjointnessDigest`, plus `externalOwnershipEvidence:
ExternalSupportOwnershipEvidence`; `preArmExternal` requires
`pendingSupportActionId`, `pendingSupportActionDigest`, `authorizationState:
awaitingArm | frozenForRecovery`, `freezeKind?:
preArmCancellationEffect`, `preArmFreezeDigest?`, `armingReceiptAbsent:
true`, repository actor, both complete delta
digests, `supportTransitionsDigest`, `preserveAsExternalBaseline: true`, and the
singleton mismatch list `[armingOrderViolated]`. `freezeKind` and freeze digest
are required together exactly for `frozenForRecovery` and absent for
`awaitingArm`; `preArmFreezeDigest` hashes the frozen authorization plus
interrupted operation/cancellation identity while excluding all observations,
partitions, plans, and recovery digests, so no cycle is possible. No other
frozen kind is legal. `corrective` is itself the tagged subunion
defined below. For `authorized`/the action-correction subvariant, the working-IB
field is required in separate mode and absent in reserved mode. Those
four positive classifications require an empty mismatch list and complete
delta evidence, including the digest of an empty delta.
`preArmExternal` is intentionally not positive authorization: it may equal the
pending transition/actor/IB exactly, but because no arming receipt existed it
can only be preserved by awaiting-action cancellation or its frozen no-arming
recovery and can never be
retroactively reconciled, consumed, or frozen as `thisAuthorizedAction`.
For `authorized`, the action/arming IDs/digests and transitions digest equal the
enclosing armed/frozen authorization, the observed transitions digest equals its
authorized transitions digest, and attribution evidence binds that action plus
repository actor/version. The first-root literal is capability-proven from the
complete range after the arming cursor. Any extra root/content/support delta is invalid. A
mixed external support-and-content version is invalid/conflicting, never the
positive `externalSupport` class. Positive `corrective` content is legal only
as the exact instruction-bound restorations.

The positive `corrective` classification is a closed tagged `oneOf` of:

- `actionCorrection { correctionKind: actionCorrection, repositoryActor,
  manualTargetMode, workingInfobaseIdentity?, rootDeltaDigest,
  contentDeltaDigest, correctiveInstructionDigest }`, whose mode/IB fields and
  delta exactly match the historical `SupportCorrectiveInstruction`; or
- `externalConflictCorrection { correctionKind: externalConflictCorrection,
  repositoryActor, rootDeltaDigest, contentDeltaDigest, conflictResolutionId,
  supportConflictInstructionDigest, finalBaselineDigest,
  externalOwnershipEvidence: ExternalSupportOwnershipEvidence }`, whose IDs,
  instruction digest, actor/delta, and final baseline equal the current frozen
  conflict instruction and immutable external evidence.

Neither corrective subvariant is validated history evidence from its own shape
or `classificationDigest` alone. For `actionCorrection`, typed source resolution
loads and rehashes the exact historical `SupportCorrectiveInstruction` named by
`correctiveInstructionDigest`, then proves the repository actor, manual target
mode/working-IB presence, and complete root/content delta equal that
instruction's derived `requiredRootDeltaDigest` and
`requiredContentDeltaDigest`.
For `externalConflictCorrection`, it loads and rehashes the Task 8
`SupportConflictInstruction` named by `supportConflictInstructionDigest`, proves
`conflictResolutionId` equality, requires `finalBaselineDigest ==
requiredFinalBaselineDigest`, and validates `ExternalSupportOwnershipEvidence`
as the immutable provenance of the same repository actor/version/root/content
delta. In both leaves the independently selected historical source authority
also carries the frozen `supportActionId`, which equals the resolver's stable
action scope; this is especially required for `SupportConflictInstruction`,
whose own wire intentionally contains no action ID. The selected source
ref/index proof version equals the observation and partition-entry version, the
observation discriminator maps only to partition classification `corrective`,
its `classificationDigest` is recomputed, and `semanticDeltaDigest` uses that
exact instruction digest. An unavailable, multiple, wrong-kind,
wrong-action/conflict, cross-leaf, or digest-mismatched instruction/evidence
source leaves the partition unvalidated. Conflict-resolution IDs remain
per-version historical facts, so a later frozen retry may publish a distinct
conflict instruction without escaping the stable support-action scope.

The second subvariant represents the allowed external corrective sequence for a
conflicting support target. It is preserved and never automatically inversed;
an unmatched/partially attributed version remains conflict/inconclusive.

`invalid` is itself a closed provenance-tagged `oneOf`, always with a non-empty
mismatch list:

- `thisAuthorizedAction { provenance: thisAuthorizedAction, repositoryActor,
  manualTargetMode, workingInfobaseIdentity?, armingReceiptId,
  armingReceiptDigest, firstRootSupportAfterArming: true | false, rootDeltaDigest,
  contentDeltaDigest, actionAttributionEvidenceDigest }`, with the same
  mode-dependent working-IB presence rule; `false` requires
  `armingOrderViolated` and is never an authorized observation;
- `externalActor { provenance: externalActor, repositoryActor,
  observedWorkingInfobaseIdentity: ManualWorkingInfobaseIdentity | null,
  rootDeltaDigest, contentDeltaDigest, provenNotThisAction: true,
  externalOwnershipEvidence: ExternalSupportOwnershipEvidence }`; or
- `unattributed { provenance: unattributed, repositoryActor: null,
  rootDeltaDigest: Sha256 | null, contentDeltaDigest: Sha256 | null,
  missingEvidenceKinds: SupportMissingEvidenceKind[] }`, whose non-empty list
  contains only the actor/mode/working-IB/delta/ownership/layer/history evidence
  values of that enum and whose mismatches include
  `versionUnattributed` and whose missing list is non-empty.

The external variant is never automatically inversed. The unattributed variant
is never an automatic restoration target: while the authorization is armed it
is inconclusive, and after a frozen chain exists it requires typed external
conflict resolution. The list
covers every version in the exact contiguous recovery cursor range, including
otherwise valid authorized/routine/external-support/corrective versions, and the canonical union
of only invalid-version mismatches plus any versionless original mismatch equals
the stop's top-level `mismatchKinds` exactly.
For every top-level leaf and every `corrective`/`invalid` subleaf,
`classificationDigest ==
sha256(canonical(observation-without-classificationDigest))`; only that one
top-level member is removed. The input contains no partition,
`sourceEvidenceRef`, or `semanticDeltaDigest`, so this content address has no
digest cycle. It binds actor/mode/provenance, mismatch list, delta evidence, and
every variant-specific ownership/correction field.
Observation classifications map exactly to partition entries: routine relevance
selects `unrelatedRoutine`/`relevantRoutine`; `authorized` selects
`authorizedSupport`; the other names map directly. Each observation derives the
canonical `semanticDeltaDigest` envelope defined by
`RepositoryHistoryPartition`, and the corresponding partition entry must equal
it byte-for-byte.
Disjoint `externalSupport` is preserved in every destination baseline and
always selects the relevant-advance phase/fresh preflight. A wrong-actor/target
version whose capability-proven external delta is byte-for-byte the authorized
transition is an `invalid` observation with `provenance: externalActor` and
selects `preserveExternalAndReauthorize`; it is preserved, not accepted as this
action. Any other overlapping support delta, or one without proven external
attribution, is a `supportPrerequisiteConflict`, never routine, authorized, or
an automatic restoration target.
`SupportContentRestoration` is a closed `oneOf` of `{ action: restoreExisting,
targetKind: configurationRoot, objectDisplay, correctionBaseCursor,
expectedCurrentFingerprint, expectedRepositoryFingerprint,
correctionLockObjectIds: [], finalizationLockObjectIds: [],
structuralConfirmationRequired: false }`, `{
action: restoreExisting, targetKind: developmentObject, objectId, objectDisplay,
correctionBaseCursor, expectedCurrentFingerprint,
expectedRepositoryFingerprint,
correctionLockObjectIds[], finalizationLockObjectIds[],
referenceClosureDigest, structuralConfirmationRequired: false }`, `{
action: removeUnauthorizedAddition, targetKind: developmentObject, objectId,
objectDisplay, correctionBaseCursor, expectedCurrentFingerprint,
expectedAbsent: true,
correctionLockObjectIds[], finalizationLockObjectIds[],
referenceClosureDigest, structuralConfirmationRequired: true,
structuralCapabilityRowId }`, or `{ action: recreateUnauthorizedDeletion,
targetKind: developmentObject, objectId, objectDisplay,
correctionBaseCursor, expectedCurrentAbsent: true,
expectedRepositoryFingerprint, sourceCheckpointId,
correctionLockObjectIds[], finalizationLockObjectIds[],
referenceClosureDigest, structuralConfirmationRequired: true,
structuralCapabilityRowId }`. Both lock-ID lists contain the exact existing
parent/referrer/development-object closure in canonical order; an added/absent
object itself need not be lockable. For removal, the existing unauthorized
object may appear only in the correction set; for recreation, the newly
recreated object may appear only in the finalization set. Parent/referrer
closures appear in both as required by existence. `unauthorizedContentChanged` covers every
non-support content delta, including an unrelated property of the configuration
root, add, or delete; it is not limited to non-root modifications.
`SupportRecoveryLockTarget` is a closed
`oneOf` of `{ targetKind: configurationRoot, objectDisplay, reasons:
RepositoryUpdateLockReason[] }` or `{ targetKind:
developmentObject, objectId, objectDisplay, reasons:
RepositoryUpdateLockReason[] }`. Reason lists
are non-empty, deduplicated, and canonical. The root target has no `objectId`, is
the unique first entry, always includes `supportGraphGuard`, and otherwise has
only its exact update/structural-closure roles. Following targets are exactly the canonical union of every
restoration's applicable correction or finalization lock-ID list; no subtree
approximation is allowed.
`SupportRecoveryTransition` is a closed `oneOf` of every ordinary
`SupportTransition` plus `restoreVendorConfigurationSupport { targetKind:
configurationRoot, configurationDisplay, layerId, fromState: offSupport,
toState: locked | editable, vendorDistributionArtifactId,
recoveryDistributionHandoffId, capabilityRowId }` or
`restoreVendorObjectSupport { targetKind: developmentObject, objectId,
objectDisplay, layerId, fromState: offSupport, toState: locked | editable,
vendorDistributionArtifactId, recoveryDistributionHandoffId,
capabilityRowId }`. The root variant has no
`objectId`; the object variant requires it and maps one-to-one to the same exact
support-graph observation; vendor-support restoration itself is a root support-
settings change and does not create a development-object lock target. Either
recovery-only variant is legal only when a verified vendor distribution
exactly matches the support layer, is present in the frozen authorization's
`supportRecoveryDistributions`, and the capability row proves restoration;
its handoff ID is the same evidence record's proven user-visible handoff;
neither is accepted by an ordinary support-action authorization or general
support-edit tool.
`SupportCorrectiveInstruction` is closed `{ kind:
correctSupportPrerequisite, supportActionId, purpose, manualTargetMode,
repositoryUsername, workingInfobaseIdentity?: ManualWorkingInfobaseIdentity,
correctionBaseCursor: RepositoryHistoryCursor,
correctionLockTargets: SupportRecoveryLockTarget[],
finalizationLockTargets: SupportRecoveryLockTarget[], requiredRootTransitions:
SupportRecoveryTransition[],
requiredContentRestorations: SupportContentRestoration[],
requiredRootDeltaDigest, requiredContentDeltaDigest,
distributionHandoffs: SupportRecoveryDistributionHandoff[],
handoffRevalidations: SupportRecoveryHandoffRevalidation[],
desiredSupportGraphDigest, desiredRepositoryContentDigest,
offSupportForbidden: true, commitAsSeparateRecoveryVersion: true,
releaseAllLocks: true, resumeWith: branched.status,
correctiveInstructionDigest }`. It is root-only when the
content-restoration list is empty. Both lock lists are non-empty and start with
the root guard. The correction list is the canonical union of correction-time
closures; the finalization list is the canonical union for the desired
post-correction existence state. Each contains only the exact exceptional
objects/parents/referrers, never a broader subtree. The working-IB field
follows the manual-target-mode presence rule. The recovery tool never executes
this external instruction or exposes credentials/commands. The handoff list is
the canonical deduplicated set referenced by recovery transitions, is
byte-identical to the frozen authorization evidence, and is empty when no
off-support restoration requires a vendor CF. The revalidation list maps
one-to-one to that non-empty handoff subset and proves same-SHA availability and
retention immediately before this instruction.
`requiredRootDeltaDigest == sha256(canonical({ requiredRootTransitions }))`
over the named closed `SupportRequiredRootDeltaDigestRecord`, and
`requiredContentDeltaDigest == sha256(canonical({
requiredContentRestorations }))` over the named closed
`SupportRequiredContentDeltaDigestRecord`. They are derived by the instruction
constructor, not supplied as independent authority. They bind the exact version
delta expected from the human corrective commit. The separate
`desiredSupportGraphDigest` and `desiredRepositoryContentDigest` remain endpoint
state digests and must never be compared to a delta digest.
`correctiveInstructionDigest ==
sha256(canonical(instruction-without-correctiveInstructionDigest))`. A
`corrective` history observation repeats the digest of the exact instruction
that was current when its version was authored; replaced/wrong instructions
remain immutable audit records, so later recomputation cannot reattribute an
old version.
`desiredRepositoryContentDigest` is computed after applying every valid routine,
proven disjoint `externalSupport`, and disposition-preserved external-actor
version in repository order and then inversing only this authorized action's
invalid content deltas; it never rolls
the repository back wholesale to the action anchor or removes another actor's
support state.
Every restoration's `correctionBaseCursor` equals the instruction cursor. After
acquiring the canonical root-first lock closure, recovery rechecks each expected
current presence/fingerprint before the human correction is accepted; drift
invalidates and recomputes the instruction instead of overwriting newer work.

This correction is a separate frozen-recovery authorization, not a widening of
the normal manual window. In `reservedOriginal` it may use the reserved account
only for the root guard plus the exact digest-bound restoration targets and must
restore the actor's empty lock inventory before reconciliation. In
`separateWorkingInfobase` it binds the same exact human actor/working-IB identity.
Either mode requires a capability row for the recovery transition and lock
scope. No corrective instruction can make a tainted task eligible for successful
integration.

`SupportRecoveryDesiredTarget` is a closed tagged `oneOf` of `rootPresent {
targetKind: configurationRoot, state: present, objectDisplay, desiredFingerprint }`,
`objectPresent { targetKind: developmentObject, state: present, objectId, objectDisplay,
desiredFingerprint }`, or `objectAbsent { targetKind: developmentObject,
state: absent, objectId, objectDisplay, expectedAbsent: true }`. It expresses the approved
destination without inventing the repository version of a future corrective
commit.

`SupportRecoveryFinalizationPlan` is closed `{ disposition:
SupportRecoveryDisposition, lockTargets: SupportRecoveryLockTarget[],
desiredTargets: SupportRecoveryDesiredTarget[],
historyFromCursor: RepositoryHistoryCursor, materializedSelectiveUpdatePlan?:
SelectiveRepositoryUpdatePlan, desiredSupportGraphDigest,
desiredRepositoryContentDigest, planDigest }`. It exists for every frozen
support-recovery plan, even when no corrective instruction is currently
needed. Its root-first lock set and desired targets describe the exact
post-correction existence state. `historyFromCursor` equals the frozen
authorization's `expectedBeforeHistoryCursor`; materialization cannot advance
or replace that start anchor, and the arming receipt remains its exact prefix.
`desiredTargets` is canonical and unique by
target identity, starts with exactly one configuration root, and never contains
both present/absent or differing fingerprints for one object. Once history
materializes the plan, each desired-present target maps to the exact
`rootPresent`/`objectPresent` repository version/fingerprint and each
desired-absent target maps to `objectAbsent` at the observed absence version;
the materialized plan has no extra/missing target. `materializedSelectiveUpdatePlan` is absent
while any corrective/conflict evidence is pending. It becomes required only
after the complete observed history proves a current repository state/version
for every desired target; its planned target locators/presence/fingerprints and
generic lock coverage equal the desired/finalization sets. Materialization or
any newly observed history before effect intent while authorization remains
unchanged recomputes `planDigest`, invalidates the prior approval/latest guard
proof, and requires a fresh approval before locking. The bounded post-release
partition observed after durable terminalization is receipt/result-phase
evidence: it cannot retroactively invalidate the consumed approval. The scan
ends before a first disallowed successor or at a capability-proven history-
coverage gap, records the corresponding `classified`/`unclassified` or
`coverageUnknown` `DeferredRepositoryAdvance`, completes the terminal receipt, and gates the next
call to routine update; it never reopens the authorization. When a
`SupportCorrectiveInstruction` exists, its `finalizationLockTargets` and desired
digests equal this plan byte-for-byte; the selective update plan's generic lock
targets, when materialized, have the same target kind/object ID/display/reason
records byte-for-byte in the same order.
Its structural flag is true only for an exact approved add/delete restoration,
and its conditional capability row equals that restoration's frozen structural
capability; a support/root-only finalization keeps the flag false.
Correction-time targets may differ.
`planDigest == sha256(canonical(plan-without-planDigest))`, including explicit
JSON `null` for the absent materialized plan.

`SupportRecoveryGuardProof` is a closed tagged `oneOf`:

- `blockedBeforeRoot { outcome: blockedBeforeRoot, guardReceiptId, manualTargetMode,
  finalizationPlanDigest, plannedLockTargets, acquiredInOrder: [], failedTarget:
  RepositoryTargetIdentity, failedTargetDisplay: RepositoryTargetDisplay,
  lockedBy: RepositoryOwnerIdentity | null, authorizationOutcome: unchanged,
  releasedInReverseOrder: [], releaseVerified: true, proofDigest }`;
- `blockedAfterPartial { outcome: blockedAfterPartial, guardReceiptId, manualTargetMode,
  finalizationPlanDigest, plannedLockTargets, acquiredInOrder[], failedTarget:
  RepositoryTargetIdentity, failedTargetDisplay: RepositoryTargetDisplay,
  lockedBy: RepositoryOwnerIdentity | null, authorizationOutcome: unchanged,
  releasedInReverseOrder[], releaseVerified: true, proofDigest }`;
- `stoppedAfterCompleteGuard { outcome: stoppedAfterCompleteGuard, guardReceiptId, manualTargetMode,
  manualActorLockInventoryProof?: ManualActorLockInventoryProof,
  reservedOriginalLeaseStopEvidence?: ReservedOriginalLeaseStopEvidence,
  finalizationPlanDigest, plannedLockTargets, acquiredInOrder[],
  historyFromCursor, historyThroughCursor, historyPartitionDigest,
  supportGraphRecheckedUnderGuard: true,
  correctiveBeforeStateBindingVerified: true,
  contentRecheckedUnderGuard: true, originalRecheckedUnderGuard: true,
  selectiveUpdatePerformed: false, authorizationOutcome: unchanged,
  releasedInReverseOrder[], releaseVerified: true, proofDigest }`; or
- `completed { outcome: completed, guardReceiptId, manualTargetMode,
  manualActorLockInventoryProof?: ManualActorLockInventoryProof,
  reservedOriginalTerminalizationProof?:
  ReservedOriginalTerminalizationProof,
  finalizationPlanDigest, plannedLockTargets, acquiredInOrder[],
  historyFromCursor, historyThroughCursor, historyPartitionDigest,
  supportGraphRecheckedUnderGuard: true,
  correctiveBeforeStateBindingVerified: true,
  contentRecheckedUnderGuard: true, originalRecheckedUnderGuard: true,
  selectiveUpdateProof: SelectiveRepositoryUpdateProof,
  postReleaseObservedHistoryCursor: RepositoryHistoryCursor,
  postReleaseHistoryPartition: RepositoryHistoryPartition,
  deferredRepositoryAdvance?: DeferredRepositoryAdvance,
  authorizationOutcome: cancelled | abandonmentFinalized,
  releasedInReverseOrder[], releaseVerified: true, proofDigest }`.

No guard attempt may begin until the finalization plan has a materialized
selective update plan and, in separate mode, the working-IB closure plan is also
materialized; `finalizationPlanDigest` always equals its current
`planDigest`. The planned targets equal the persisted finalization lock set
byte-for-byte and the root is first. For `blockedBeforeRoot`, `failedTarget` is
the `RepositoryTargetIdentity` of that first root and `failedTargetDisplay` is
presentation only; `guardReceiptId` is the journaled attempt receipt, not an
acquisition claim. For `blockedAfterPartial`, `acquiredInOrder` is a non-empty
proper prefix and `failedTarget` is exactly the next
`RepositoryTargetIdentity` with its presentation-only display. In both blocked
variants the release list is the exact reverse acquired prefix. For both
complete-guard variants, `acquiredInOrder` equals the whole planned list and the
release list is its exact reverse. Only complete-guard variants may carry history/content/actor
rechecks. In those variants the actor-inventory proof is required exactly for
`reservedOriginal` and absent in separate mode. The reserved-original
terminalization proof is required exactly for a completed reserved-mode guard
and absent otherwise; it binds the authorization's identity/capability and its
lease is held from the final original inspection through durable
cancel/abandon-finalize. The reserved lease-stop evidence is required exactly
for a stopped reserved-mode guard and absent otherwise. `completed` selectively updates
  only the plan's exact target set while its existing root/target/structural
  lock closure is held, never a global version, then finalizes the
  authorization. Their history endpoints/digest equal the current recovery
partition. In `completed`, `selectiveUpdateProof.observedBeforeCursor ==
historyThroughCursor`; the post-release partition starts there and ends at
`postReleaseObservedHistoryCursor`, with the proof's after cursor inside that
  contiguous allowed range. `deferredRepositoryAdvance` is present iff the
  scan encountered a first disallowed post-terminal entry or could not prove
  complete history coverage beyond the durable terminal endpoint; its `fromCursor`
  equals the post-release cursor/partition endpoint and terminal completion is
  still durable. Its selective proof's `planDigest`, selective and
  conditional structural capabilities/flags, planned targets, and generic lock
  coverage equal the materialized plan byte-for-byte.
`stoppedAfterCompleteGuard` is the complete-guard no-terminalization proof only
for the named separate-working-IB lease/dirty stop or the capability-proven
reserved-original lease-busy stop. Repository/support
destination or history drift recomputes the plans and returns correction or
reapproval with this released attempt in prior audit evidence, not as the
current latest proof. Unknown acquisition, update, finalization, or release remains
`recoveryRequired`.
Every guard `proofDigest == sha256(canonical(proof-without-proofDigest))`.

Task 9 keeps the corrective semantic authority and the finalization/guard and
working-IB closure/proof/stop output records non-deserializable, with test-only
semantic constructors. `SupportCorrectiveInstruction` and
`SupportConflictInstruction` are strict-decodable only inside an independently
selected historical source, with the latter additionally requiring the external
history-order authority; successful wire decoding remains evidence, never
authority by shape. Task 9 intentionally exposes no production corrective,
materialization, execution, completion, or stop mint. The single authority
appears only with `RecoveryPlanStatus` in Task 11, where it atomically binds the
frozen action ID/digest, exact approved history partition, capability-proven
materialized working-IB cursor/object map, materialized selective-update plan,
typed acquire/release receipt window, current under-guard destination rechecks,
and the capability-scanned post-release tail. Adding a production plan
constructor alone must not make any Task 9 fixture constructor callable.

`ManualWorkingInfobaseClosurePlan` is a closed tagged `oneOf` of:

- `desired { state: desired, workingInfobaseIdentity:
  ManualWorkingInfobaseIdentity, authorizationBaselineDigest: Sha256,
  desiredBaseFingerprint, desiredObjectFingerprintMapDigest,
  desiredSupportGraphDigest, exclusiveLeaseCapabilityId,
  cleanStateMustBeReproduced: true, planDigest }`; or
- `materialized { state: materialized, workingInfobaseIdentity:
  ManualWorkingInfobaseIdentity, authorizationBaselineDigest: Sha256,
  desiredBaseFingerprint, desiredObjectFingerprintMapDigest,
  desiredSupportGraphDigest, workingInfobaseBaseCursor:
  RepositoryHistoryCursor, recordedObjectVersionMapDigest: Sha256,
  exclusiveLeaseCapabilityId, cleanStateMustBeReproduced: true,
  planDigest }`.

`ManualWorkingInfobaseClosureProof` is closed `{
workingInfobaseIdentity: ManualWorkingInfobaseIdentity,
authorizationBaselineDigest: Sha256,
planDigest, exclusiveLeaseCapabilityId,
exclusiveLeaseReceiptId, exclusiveLeaseReleaseReceiptId,
leaseHeldThroughInspectionAndTerminalization: true,
workingInfobaseBaseCursor: RepositoryHistoryCursor, finalCurrentFingerprint,
recordedObjectVersionMapDigest: Sha256,
finalBaseFingerprint, finalObjectFingerprintMapDigest,
currentEqualsRecordedBase: true,
finalSupportGraphDigest,
noLocalSupportDelta: true, noUncommittedConfigurationDelta: true,
leaseReleased: true, leaseReleaseVerified: true,
proofDigest }`. The plan is required in every separate-mode terminalization
preview/recovery plan and the proof in completed reconciliation, cancellation,
or frozen recovery; both are absent in `reservedOriginal`. A proof is legal only
for the same acquisition/release receipt pair; both IDs are covered by
`proofDigest` and cannot be spliced across attempts. It is legal only for a
`materialized` plan. Apply uses the internal service
endpoint to acquire a capability-proven exclusive configuration lease, inspects
the working IB while edits are frozen, and terminalizes only when its current
configuration equals its own capability-proven recorded repository base with no
local delta. It need not ingest later unrelated repository versions. The base
fingerprints and support graph are derived without inventing a future repository
version, so a busy IB or pending corrective version is representable without
invented observations. A `desired` plan becomes `materialized` only after the
complete observed history provides the exact base cursor and object-version map;
that changes both plan and recovery digests, clears prior approval/latest guard
proof, and requires the same fresh reapproval as finalization-plan
materialization. The materialized base cursor is capability-proven to be either
the authorization baseline cursor or a cursor in the enclosing contiguous
history prefix through its final classified cursor; it is never required to
precede a reconciliation/corrective version that it records. The object-version
map, rather than a requested global cutoff, binds the base actually recorded by
that IB. The lease is held until
authorization consumption/cancellation/finalization is durable. A dirty external human IB is never
reset or discarded automatically; it returns `manualSupportLocalChangesRemain`
after verified lease release. A snapshot without the lease is never sufficient.
Once a support authorization exists in any non-terminal state, an unknown
working-IB lease acquire/inspection/release outcome always creates or preserves
`RecoveryPlanStatus { target: supportPrerequisite, ... }` for an armed action,
or the no-arming `target: preArmSupportCancellation` variant when it interrupts
an awaiting-action cancellation; it can never use
`target: manualWorkingInfobaseLease` or be treated as a retryable busy/dirty
stop. `manualWorkingInfobaseLease` is reserved solely for a failed/unknown
pre-authorization baseline lease, when no support action ID/digest has yet been
published.
After lease release, any new human edit is a new out-of-band event rather than
an orphaned pre-terminalization delta. The base cursor/object-version map and
the enclosing classified range/partition are covered by both digests; a fingerprint from
an unbound repository base cannot close the manual window.
For every variant, `authorizationBaselineDigest` equals the bound
`SupportActionAuthorizationData.manualWorkingInfobaseBaseline.baselineDigest`.
Its `workingInfobaseIdentity` and `exclusiveLeaseCapabilityId` equal the same
baseline fields byte-for-byte; the proof repeats those values and cannot switch
to another IB or lease capability.
For cancellation with no action version, every desired field equals that
authorization baseline and the plan can materialize from its cursor/map.
Reconciliation derives the desired terminal root/object/support expectation by
applying the one authorized version plus preserved external root versions to
that baseline; frozen recovery additionally applies its complete
corrective/preserved target sequence, while remaining `desired` until those
future versions are observed. No routine non-target tail is silently assumed to
have been ingested by the human IB.
The proof's `planDigest`, capability ID, authorization-baseline digest, base cursor/map, final base
fingerprint, final object-fingerprint map, and support graph equal the
materialized plan's values byte-for-byte:
`finalBaseFingerprint == desiredBaseFingerprint`,
`finalObjectFingerprintMapDigest == desiredObjectFingerprintMapDigest`, and
`finalSupportGraphDigest == desiredSupportGraphDigest`;
`planDigest` and `proofDigest` cover every
field of their respective closed records. A busy or dirty observation does not
mutate the plan; the next approved attempt must reproduce it.

`ManualWorkingInfobaseStopEvidence` is a closed tagged `oneOf` of
`leaseBusy { cause: leaseBusy, workingInfobaseIdentity, leaseOwner:
RepositoryOwnerIdentity | null, closurePlanDigest,
exclusiveLeaseCapabilityId, leaseBusyEvidenceDigest,
exclusiveLeaseAcquired: false }` or `leaseAcquiredDirty { cause:
leaseAcquiredDirty,
workingInfobaseIdentity, closurePlanDigest, exclusiveLeaseCapabilityId,
expectedRepositoryFingerprint,
observedWorkingInfobaseFingerprint, observedSupportGraphDigest,
exclusiveLeaseReceiptId, exclusiveLeaseReleaseReceiptId,
workingInfobaseLeaseReleased: true,
workingInfobaseLeaseReleaseVerified: true, stopEvidenceDigest }`. The busy variant is the
capability-proven clean rejection of lease acquisition, usually an open
Designer/session or another service lease; it carries no invented inspection
fingerprint or lease receipt. The dirty variant is legal only after guarded
inspection and verified release; its two receipt IDs identify that exact
acquire/release window and cannot be spliced across attempts. Neither permits terminalization or automatic
reset. Both variants' plan digest, IB identity, and capability ID equal the
current materialized closure plan byte-for-byte; the dirty expected fingerprint
equals that plan's `desiredBaseFingerprint`. Their evidence digest covers every
field in the closed variant except itself.

`SupportRecoveryExternalAction` is the closed tagged union of
`SupportCorrectiveInstruction`, `ReleaseRepositoryLocksInstruction`, or
`CleanManualWorkingInfobaseInstruction`,
`CloseReservedOriginalDesignerInstruction`, `SupportConflictInstruction`, or
`SupportEvidenceInstruction`.
Exactly one is present when frozen
recovery is waiting for a corrective version, a known foreign lock, human-IB
closure, reserved-original Designer closure, or external-support conflict
resolution respectively; it pairs with
`awaitExternalSupportCorrection`, `awaitExternalLockRelease`,
`awaitManualWorkingInfobaseClosure`, `awaitReservedOriginalClosure`, or
`awaitExternalSupportConflictResolution`, or `awaitSupportRecoveryEvidence`
byte-for-byte. Unknown effects remain
recovery rather than an external-action guess.

`SupportOwnershipReclassification` is closed `{ repositoryVersion,
priorClassification: invalid, priorProvenance: unattributed,
priorClassificationDigest, priorRootDeltaDigest: Sha256 | null,
priorContentDeltaDigest: Sha256 | null,
newObservation: SupportPrerequisiteVersionObservation,
acceptedOwnershipEvidence: ExternalSupportOwnershipEvidence }`. The new
observation has the same repository version and unchanged raw root/content delta
digests, and is only `invalid/provenance: externalActor` or positive
`externalSupport` as proven by the accepted evidence. The old unattributed
observation remains immutable audit evidence. An already proven
`thisAuthorizedAction`, `externalActor`, authorized/routine/corrective, or any
other positive classification can never be reclassified; action-owned
off-support/content taint therefore permanently retains
`restoreThenAbandon` precedence.

`SupportRecoveryReapprovalReason` is a closed tagged `oneOf` of:

- `historyAppended { cause: historyAppended, appendedObservations:
  SupportPrerequisiteVersionObservation[] }`, with a non-empty exact contiguous
  suffix of the new recovery history;
- `ownershipReclassified { cause: ownershipReclassified,
  reclassifications: SupportOwnershipReclassification[] }`, with a non-empty
  canonical list; or
- `workingInfobaseClosureMaterialized { cause:
  workingInfobaseClosureMaterialized, previousClosurePlanDigest,
  materializedClosurePlanDigest, closureMaterializationEvidenceDigest }`, legal
  only for separate mode and binding the capability-proven base cursor/map
  observation.

The reapproval reason list is non-empty and canonical. It may contain multiple
different causes from one observation boundary but never duplicate a cause.
Thus an ownership receipt or working-IB materialization can change a frozen
digest without fabricating a new repository version.

`CommonTaskRequest` is the closed `{ cwd, taskId }` record. `cwd` must resolve
to the workspace whose `unica.local.yaml` contains the task's `projectId`; it
never selects the disposable workspace or an external artifact.
`CommonMutationRequest` is the closed intersection of `CommonTaskRequest` with
`{ operationId }`. Every per-tool request list below enumerates only its
tool-specific fields and is normatively intersected branch-by-branch with the
common record selected by execution policy: `readOnly` uses
`CommonTaskRequest`; every mutating policy uses `CommonMutationRequest`.
Strict tagged unions remain strict after this intersection. The sole mixed
stage exception is `supportPrerequisiteArm`: `armPreview` intersects
`CommonTaskRequest` and therefore still has `cwd`/`taskId` but no
`operationId`/`dryRun`; `armApply` intersects `CommonMutationRequest`. A phrase
“no fields beyond common” means the tool-specific record is empty, not that the
common fields disappear.

Compatible general Unica tools use an additional exact context instead of a
task path. It is a closed `oneOf`:

```json
{
  "cwd": "/original/project",
  "branchedTask": {
    "taskId": "TASK-142",
    "taskWorkspaceId": "381f2188-554e-4abf-8cba-495a4570c5cd"
  }
}
```

or, only for a prepared manual/combine conflict workspace:

```json
{
  "cwd": "/original/project",
  "branchedTask": {
    "taskId": "TASK-142",
    "mergeResolution": {
      "sessionId": "1f99e816-1d18-4265-bdf0-204918f47c48",
      "workspaceId": "73c40b15-1ca0-4cbb-a8a8-01b28d38bfd5",
      "expectedBaseSessionDigest": "3333333333333333333333333333333333333333333333333333333333333333"
    }
  }
}
```

Their descriptors explicitly declare `supportsBranchedTask`; a tagged general
tool also declares the exact supported operation variants. The generated
registry snapshot is the release manifest for this compatibility, so enabling
one safe `runtime.execute` operation cannot silently enable all variants. The
application resolves the ID through durable state, verifies the owned marker,
phase, and leases, and supplies the disposable `WorkspaceContext` internally.
Compatible reads accept `branchedTask` but require neither `operationId` nor a
change receipt. Compatible mutation requests accept it and additionally require
`operationId`; their closed completed response requires `data.changeReceipt:
BranchedChangeReceipt` plus `data.cacheImpact` for the resolved workspace.
The receipt's `mutationOutcome` is also the completed result's authoritative
changed/no-change discriminator: every tool-specific change indicator and
`cacheImpact` branch must agree with it. Neither field is mutation input.

`BranchedAffectedTarget` is the closed `targetKind`-tagged `oneOf` of
`metadataProperty { targetKind: metadataProperty, objectId: MetadataObjectId,
propertyPath }` or `supportLayerProperty { targetKind: supportLayerProperty,
layerId, objectId?: MetadataObjectId, propertyPath }`. `propertyPath` is a
non-empty canonical metadata/support path. The support object is present for an
object-state change and absent for a layer-capability change; no other optional
shape is legal.

`CompatibleTaskMutationPhase` is the closed enum `developing`,
`localVerified`, `synchronizationPrepared`, `synchronizationConflicts`,
`synchronized`, `integrationPlanned`, `blockedByForeignLock`,
`unexpectedDelta`, or `validationFailed`; the last literal additionally
requires the existing safe-rollback/no-original-difference/no-owned-lock
precondition. `TaskWorkspaceChangedPhaseTransition` is a closed
`phaseBefore`-tagged `oneOf` containing exactly one branch per that enum value,
each `{ phaseBefore: <literal>, resultingPhase: developing }`.
`TaskWorkspaceNoChangePhaseTransition` is a separate closed
`phaseBefore`-tagged `oneOf` containing exactly the nine equal pairs
`{ phaseBefore: <literal>, resultingPhase: <same literal> }`; using two
independent enum fields is forbidden. `MergeResolutionPhaseTransition` is the
closed singleton `{ phaseBefore: synchronizationConflicts, resultingPhase:
synchronizationConflicts }`. Thus the response schema itself cannot represent
an illegal input phase or claim a different phase for an idempotent result.

`BranchedChangeReceipt` is a closed outer `contextKind`-tagged `oneOf`; each
outer branch is itself a closed `mutationOutcome`-tagged `oneOf`, yielding
exactly these four leaves:

- `TaskWorkspaceChangeReceipt { contextKind: taskWorkspaceChange,
  mutationOutcome: changed, changeReceiptId: UnicaId,
  affectedTargets: BranchedAffectedTarget[], beforeSha256: Sha256,
  afterSha256: Sha256, eventIds: UnicaId[], invalidatedEvidenceIds: UnicaId[],
  phaseTransition: TaskWorkspaceChangedPhaseTransition,
  changeReceiptDigest: Sha256 }`;
- `TaskWorkspaceNoChangeReceipt { contextKind: taskWorkspaceChange,
  mutationOutcome: noChange, changeReceiptId: UnicaId,
  affectedTargets: BranchedAffectedTarget[], contentSha256: Sha256,
  eventIds: [], invalidatedEvidenceIds: [],
  phaseTransition: TaskWorkspaceNoChangePhaseTransition,
  changeReceiptDigest: Sha256 }`;
- `MergeResolutionChangeReceipt { contextKind: mergeResolutionChange,
  mutationOutcome: changed, changeReceiptId: UnicaId,
  affectedTargets: BranchedAffectedTarget[], beforeSha256: Sha256,
  afterSha256: Sha256, eventIds: UnicaId[], invalidatedEvidenceIds: [],
  supersededChangeReceiptIds: UnicaId[], supersededDecisionIds: UnicaId[],
  pendingReplacementDecisionId?: UnicaId,
  decisionSetDigestBefore: Sha256, revisedDecisionSetDigest: Sha256,
  phaseTransition: MergeResolutionPhaseTransition, baseSessionDigest: Sha256,
  workspaceGenerationId: UnicaId, receiptSequence: integer,
  changeReceiptDigest: Sha256 }`;
- `MergeResolutionNoChangeReceipt { contextKind: mergeResolutionChange,
  mutationOutcome: noChange, changeReceiptId: UnicaId,
  affectedTargets: BranchedAffectedTarget[], contentSha256: Sha256,
  eventIds: [], invalidatedEvidenceIds: [], supersededChangeReceiptIds: [],
  supersededDecisionIds: [], decisionSetDigest: Sha256,
  phaseTransition: MergeResolutionPhaseTransition, baseSessionDigest: Sha256,
  workspaceGenerationId: UnicaId,
  receiptSequence: integer, changeReceiptDigest: Sha256 }`.

All four leaves require a non-empty, canonical, duplicate-free target list.
Each merge-resolution leaf requires a singleton `metadataProperty` target equal
to the exact object/property identity addressed by the compatible mutation;
support-layer targets are forbidden there. A `changed` leaf requires
`beforeSha256 != afterSha256`; those hashes cover the exact target list's
canonical content projection immediately before and after the mutation. A
`noChange` leaf instead requires only `contentSha256`, equal to both the
pre-call and post-call projection, and forbids both before/after fields. Changed
leaves have a non-empty canonical duplicate-free `eventIds`; no-change leaves
have the literal empty list because the operation journal record is not a
domain event. Every `changeReceiptDigest ==
sha256(canonical(receipt-without-changeReceiptDigest))`, including the complete
supersession list.

For `TaskWorkspaceChangeReceipt`, `invalidatedEvidenceIds` is exactly the
canonical ID projection of every live durable evidence record whose direct or
transitive semantic inputs contain the pre-mutation task-source fingerprint and
which the same atomic transition marks invalid. No unrelated evidence,
immutable audit receipt, or terminal recovery/support receipt may appear; the
list is empty exactly when no such live descendant existed, and no such
descendant may remain live after the transition. The resulting phase is always
`developing` through its exact transition branch; all session/generation/
sequence/supersession/decision-set fields are absent.
`TaskWorkspaceNoChangeReceipt` selects the one transition branch whose
`phaseBefore` equals the durable pre-call task phase and therefore preserves
that phase plus every evidence/cache/status record. Both ID lists are empty and
none of those resolution-only fields is legal.

For both merge-resolution leaves, `invalidatedEvidenceIds` is the literal empty
list: resolution mutations never hide decision/receipt-head changes in a generic
evidence-invalidation list. A changed leaf instead records in
`supersededChangeReceiptIds`, in ascending `receiptSequence`, exactly the prior
currently selectable changed receipts in the same live generation with the
same singleton target. Their handles are atomically marked as superseded by the
new receipt. Consumed receipts, receipts already superseded by an earlier
receipt, and every different-target receipt are excluded. A no-change receipt
always has empty receipt/decision supersession lists and changes no handle.

A merge-resolution changed leaf also records `supersededDecisionIds` as either
the empty list or the singleton current decision head for the conflict with
that exact target at the mutation's linearization point. For the singleton
case, the same atomic transition removes that head from the current decision
map and marks only its derived status as superseded by this change receipt.
`pendingReplacementDecisionId` is then that singleton ID. When an earlier
changed receipt for the target had already created a replacement-pending head,
the new receipt has an empty `supersededDecisionIds` but carries that same
pending ID forward while making itself the current replacement cause. It is
absent exactly when the conflict has neither a current nor a pending historical
decision head. `decisionSetDigestBefore` and `revisedDecisionSetDigest` hash the
closed per-conflict state `undecided | current { decisionId } |
replacementPending { decisionId, causedByChangeReceiptId }`; they differ
exactly when `pendingReplacementDecisionId` is present and otherwise are equal.
A previously consumed receipt underlying that
historical decision remains immutable with `consumed: true`, is absent from
`supersededChangeReceiptIds`, and is never made selectable again. Decisions and
receipts for other targets are unchanged. A no-change leaf instead carries the
single current `decisionSetDigest` and cannot revise the active decision map.
Resolution mutations and `merge.resolve` serialize through the same durable
session CAS. Whichever transition linearizes first fixes the receipt's before/
after digests; a concurrently submitted decision with the older
`expectedDecisionSetDigest` is rejected before effect, so two current heads or
a lost replacement cannot result.
`baseSessionDigest` equals the session's immutable base and
`workspaceGenerationId` equals its active resolution-workspace generation.
`receiptSequence` is unique for every distinct changed or no-change receipt and
strictly increasing on first registration within that generation; its allowed
range is `1..9007199254740991`. Operation replay returns the same complete
receipt and sequence. `resultingPhase` is absent and task sources/phase cannot
change outside the exact `MergeResolutionPhaseTransition` pair.

Schema tests reject a missing/unknown outer or inner tag, a leaf with a field
from another leaf, an optional-field union matching both/neither outcome,
before/after fields on a no-change leaf, a `contentSha256` on a changed leaf,
non-empty no-change event/evidence/supersession lists, a missing or structurally
duplicate target/ID, more than one superseded decision, any phase pair outside
the exact closed transition union, and an out-of-range sequence. Draft 2020-12
cannot compare sibling values, prove semantic ordering/completeness, query
durable state, or recompute SHA-256; the generated schema is intentionally only
a structural superset for those invariants. Strict wire promotion and
authority-backed contract tests additionally reject equal changed hashes,
semantic duplicate or noncanonical targets/IDs, a resolution receipt with a
non-singleton/wrong-kind/wrong-object/property target, an incomplete or
misordered receipt-supersession set, an omitted/extra/wrong current or pending
decision head, a singleton superseded decision unequal to the pending ID,
unequal changed decision-set digests with no pending decision or equal digests
with one, a sequence reused by a different receipt, a stale base/generation,
and any `changeReceiptDigest` mismatch.

Before a compatible general-tool result crosses the public MCP boundary, a
recursive branched-result projection examines every field, including
`changes`, `artifacts`, `summary`, `warnings`, `errors`, `evidence`, `data`, and
`cache`. Task/work-root/state/coordination absolute paths are replaced with
task-workspace-relative logical paths or registered artifact IDs and roles;
secret values and path-bearing platform diagnostics are redacted. No absolute
disposable path may remain in either a structured value or free text. A tool
whose closed response cannot be projected without losing required semantics
must not declare `supportsBranchedTask` and returns
`toolNotBranchedCompatible` before dispatch.

A changed task-workspace mutation in `developing` keeps that phase. In
`localVerified`, `synchronizationPrepared`, `synchronizationConflicts`,
`synchronized`, `integrationPlanned`, `blockedByForeignLock`,
`unexpectedDelta`, or safely rolled-back `validationFailed`, it atomically
returns to `developing` and invalidates all
descendant checkpoints, artifacts, sessions/decisions, verifications,
support gates/actions, integration/lock plans, and previews. It is rejected while a worker, owned lock,
original difference, or unknown effect exists and in every commit/archive/
cleanup terminal phase. A task-workspace `noChange` is a successful
idempotent observation, not an applied edit: it preserves the phase, all
descendant evidence, cache lineage, and status handles through the exact equal
`phaseTransition` branch. A phase outside `CompatibleTaskMutationPhase` is
rejected before dispatch and can never be laundered into a no-change success.
Its `data.cacheImpact`
must select the exact zero-impact branch declared by that concrete tool's
registry-generated response schema; a compatible mutation cannot be registered
unless its no-change success has such a closed branch. A changed receipt must
select the descriptor's changed-impact branch, advance the resolved workspace's
cache generation, and agree with its exact emitted events/invalidation closure
even when no old cache entry existed.

A resolution-workspace mutation is allowed only for the named live conflict
session in `synchronizationConflicts`, carries the fixed equal phase pair,
changes no task source, and produces a receipt bound to immutable
`expectedBaseSessionDigest`. A changed result atomically supersedes only the
prior same-target selectable receipts named by its exact receipt-supersession
list and demotes only the same-conflict active decision named by its exact
decision-supersession list; a no-change result supersedes neither. The active
decision-set digest evolves in that same atomic transition. Recreating the sandbox
invalidates the workspace and makes every receipt from that generation
unselectable, without rewriting its immutable receipt body. A direct task path
in `cwd`, an unknown ID, or an incompatible tool is rejected.

On exact `operationId` replay, every compatible mutation returns the same
tool-specific payload, `cacheImpact`, receipt ID, receipt body, sequence, and
digest; replay neither emits another event nor repeats invalidation or
supersession. Reusing the ID with different canonical input returns
`operationReplayMismatch`, including when the first result was `noChange`.

The packaged full-cycle transcript requires compatible project/configuration
reads, layer-aware support mutation, the concrete typed development mutations
selected by its fixture, configuration build/load, syntax/diagnostics, and
configured test execution. It covers every general mutation category that the
package advertises and the skill selects. Every compatible BSL writer advertised
by the package is selected and exercised in a with-writer transcript; this is a
conditional package test and does not make a BSL writer a prerequisite for
package release. The without-writer transcript instead proves that the skill
stops on the missing capability before attempting BSL mutation and uses no
fallback. Each selected concrete tool or operation appears in the registry
snapshot; generic family-name claims do not satisfy compatibility.

The registry also classifies each compatible mutation as either an atomic
workspace-source mutation or an `authoritativeTaskConfigurationMutation`.
Atomic source edits use their durable change receipt. A build/load/runtime
variant that can leave the task IB partially changed uses the same
operation-worker and `taskConfiguration` recovery contract as delivery deploy
and task `merge.apply`; merely tagging the tool `supportsBranchedTask` cannot
downgrade that boundary.

`notCreated` is a read/preflight status, not a persisted task phase. `TaskPhase`
is the closed enum of these persisted task phases:

```text
created preflightPassed baselineReady developing localVerified
synchronizationPrepared synchronizationConflicts synchronized
integrationPlanned acquiringLocks locked mainMerged mainValidated committing
committedAndUnlocked archivedSuccess cleanedSuccess
blockedByForeignLock staleRelevantBaseline lockPlanExpansionRequired
staleSupportPreflight
unexpectedDelta validationFailed
commitBlocked recoveryRequired committedUnverified
abandonmentReady
archivedAbandoned cleanedAbandoned
```

`ReservationOwnerRef` is the closed tagged `oneOf` of `startAttempt {
ownerKind: startAttempt, projectId: ProjectId, taskId: TaskId, operationId:
OperationId }` or `unresolvedTask { ownerKind: unresolvedTask, projectId:
ProjectId, taskId: TaskId, instanceId: UnicaId, phase: TaskPhase }`. It is a
redacted ownership reference, never a state-root/work-root/credential locator.
`NotCreatedData` is closed `{ exists: false, startAllowed: boolean, blockers:
NotCreatedBlocker[] }`, where `NotCreatedBlocker` is the closed tagged union of
`operationInProgress { code: operationInProgress, context: OperationErrorContext }`,
`targetReservationBusy { code: targetReservationBusy, context:
TargetReservationBusyContext }`, `repositoryAccountReservationBusy { code:
repositoryAccountReservationBusy, context: RepositoryAccountReservationBusyContext }`,
`projectIdentityCollision { code: projectIdentityCollision, context:
ProfileStateErrorContext }`, and `stateRootRelocationRequired { code:
stateRootRelocationRequired, context: ProfileStateErrorContext }`. `startAllowed
== blockers.isEmpty`. It means only that authoritative coordination admits a
new start attempt; profile, secret, topology, and capability preflight can
still reject it. Reservation blocker contexts are byte-identical to the
corresponding rejected-result contexts.

## Response Envelope

Every task-bound response is one concrete instantiation of the closed generic
field contract `TaskResultFields<C, W, A, K, E, D>`. In schema algebra, not
additional wire syntax, its fields are exactly:

```text
{
  ok,
  resultKind,
  taskId: TaskId,
  status: "notCreated" | TaskPhase,
  summary: Summary,
  operationId?: OperationId,
  changes: C[],
  warnings: W[],
  errors: TaskErrorEntry[],
  artifacts: A[],
  cache: K,
  evidence: E,
  data: D,
  stopCode?: StableErrorCode
}
```

The generic parameters are compile-time/schema-generation slots and never
appear on the wire. Before a concrete result schema exists, its later owner must
bind `C`, `W`, and `A` to exact named recursively closed item schemas and bind
`K`, `E`, and `D` to exact named recursively closed, schema-bounded payload
schemas. Task 6 defines only this envelope family and its branch invariants; it
does not invent placeholder change, warning, artifact, cache, evidence, or data
records. A concrete binding may use neither `serde_json::Value`, an untyped map
or array, nor a free-form object. The three general arrays are bounded to at
most 1024 items and use their semantic validated newtype whenever they have
ordering, uniqueness, membership, or non-empty invariants. The substituted
outer and nested schemas remain closed after every `$ref` is resolved.

The final production `TaskResultEnvelope` is exactly
`ReadOnlyTaskResultEnvelope | MutatingTaskResultEnvelope`. Each side is the same
closed three-way result union and the policy distinction is physical, not an
optional-field convention:
`ReadOnlyTaskResultEnvelope` forbids top-level `operationId`, while
`MutatingTaskResultEnvelope` requires it and binds it to the request. The three
result branches are:

- `completed`: `ok: true`, `resultKind: "completed"`, empty `errors`, no
  `stopCode`, and the exact later-owned tool/variant completed-data binding for
  `D`;
- `stopped`: `ok: false`, `resultKind: "stopped"`, a required stable
  `stopCode`, an exact singleton `errors` array whose code equals `stopCode`,
  and the exact later-owned evidence-bearing stop-data binding for `D` from the
  matrix below;
- `rejected`: `ok: false`, `resultKind: "rejected"`, no `stopCode`, an exact
  singleton `errors` array, and `D = TaskErrorData { code, context,
  allowedNextActions[] }`.

All three branches physically retain `changes`, `warnings`, `artifacts`,
`cache`, `evidence`, and `data`; result kind cannot erase one or replace its
bound type with an anonymous empty object. A read-only payload may contain a
separately named, typed reference to a pre-existing active or terminal operation
inside `D`, but that never permits a top-level `operationId`.
Every concrete production constructor copies `taskId` from the validated
request; it never accepts an independently chosen response task ID. Task 16
proves `response.taskId == request.taskId` for every physical selector variant.

`resultKind` and its field invariants are the outer discriminator. A completed
classification/session/verification and its evidence-bearing stopped outcome
may intentionally use the same named domain-data schema, whose required outcome
fields distinguish the observation. A `rejected` result never borrows either
domain-data shape. `TaskErrorEntry` is the closed `{ code: StableErrorCode,
diagnostic: Diagnostic }` record. Completed `errors` is exactly `[]`; stopped
and rejected `errors` contain exactly one entry whose code respectively equals
`stopCode` or `TaskErrorData.code`. No secondary error entry is legal.
Coexisting conditions remain in typed `data`/`evidence`, never in another error
element. `context` is a named redacted closed record selected by `code`.

`StableErrorCode` is the closed enum generated from the unique literal values
in the `Code` column of the Stable Error Contract below; adding/removing a row
is a schema change. `RejectedCode` is the closed subset
`repositoryBindingMismatch`, `mainDiffersFromRepository`,
`artifactNotDistribution`, `cleanupNotAllowed`, `taskAbandonmentNotSafe`,
`operationReplayMismatch`, `recoveryPlanPending`, `taskPhaseMismatch`,
`approvalDigestMismatch`, `changeReceiptStale`,
`conflictResolutionNotAllowed`,
`adaptationDecisionAlreadyRecorded`, `taskMutationBlocked`,
`platformCapabilityUnproven`, `supportLayerAmbiguous`,
`unsupportedChangeKind`, `projectIdentityCollision`,
`stateRootRelocationRequired`, `exclusiveRepositoryUserRequired`,
`targetReservationBusy`, `repositoryAccountReservationBusy`,
`profileInvalid`, `secretUnavailable`, `stateCorrupt`,
`operationInProgress`, `taskNotFound`, `taskWorkspaceContextInvalid`,
`toolNotBranchedCompatible`, `commitCommentPolicyMismatch`, or
`integrationSetMismatch`. Stable codes outside this subset are legal only as
typed `stopped` results, not generic rejections.

Task 6 owns only this closed 30-leaf rejected vocabulary and each leaf's
code/context/action invariants; it does not make those leaves a common rejected
set for every producer. Before any production result schema or handler
registration, Task 16 fixes an explicit normative 53-by-30 matrix whose rows are
the physical lifecycle selector variants and whose columns are these rejected
leaves. Every cell is legal/illegal from that variant's stated precondition
semantics, and exact tests compare each descriptor variant's rejected union with
its row. In particular, missing-task `branched.status` is a completed
`notCreated` result, not `taskNotFound`, and start, status, and other read-only
variants do not inherit all 30 leaves. No inferred “common” fallback row is
legal.

`NextAction` is the closed tagged `oneOf` of `toolCall { actionKind: toolCall,
operation: TaskOperationSelector }` or
`externalInstruction { actionKind: externalInstruction, instructionKind:
acquireSupportRoot | releaseRepositoryLocks | performManualSupportAction |
cleanManualWorkingInfobase | closeReservedOriginalDesigner |
resolveSupportConflict | provideSupportEvidence | decideVendorRestriction }`.
`BranchedLifecycleToolName` is the closed enum of the 21 exact `unica.*`
headings in this contract. `CompatibleGeneralToolName` is a separate closed
enum generated from the public tool registry entries whose descriptor declares
`supportsBranchedTask`; `TaskOperationToolName` is their closed union.
`TaskOperationSelector` is a registry-generated closed tagged union with one
branch per `TaskOperationToolName`: `{ toolName: <literal>, requestVariant:
<that tool's literal variant> }` for multi-variant tools, or `{ toolName:
<literal> }` for a single-variant tool. Thus compatible general variants are
typed by their own descriptor and can never borrow lifecycle literals.
Task 6 adds the only action-construction API in `selectors.rs`: crate-private
constructors accept concrete typed selector/variant enums and construct
`TaskOperationSelector` values directly. Raw tool-name/request-variant strings,
`serde_json::Value`, string parsing, and serialize-then-deserialize round trips
are not legal internal construction paths; Serde remains a wire-boundary path.
The constructor match is exhaustive over the registry vocabulary and derives
its canonical ordinal from the table below rather than from lexical spelling.
The lifecycle selector vocabulary and canonical per-tool order are exactly:

| Tool | `requestVariant` values |
| --- | --- |
| `unica.branched.start` | absent |
| `unica.branched.status` | absent |
| `unica.branched.archive` | `successPreview`, `successApply`, `abandonedPreview`, `abandonedApply` |
| `unica.branched.cleanup` | `preview`, `apply` |
| `unica.delivery.inspect` | absent |
| `unica.delivery.create` | `baselineDistributionPreview`, `baselineDistributionApply`, `refreshDistributionPreview`, `refreshDistributionApply` |
| `unica.delivery.verify` | absent |
| `unica.delivery.deploy` | `preview`, `apply` |
| `unica.merge.compare` | `projectDelta`, `mainIntegration` |
| `unica.merge.prepare` | `supportedUpdate`, `supportedUpdateReplacement`, `resolvedReplay`, `mainIntegration` |
| `unica.merge.conflicts` | absent |
| `unica.merge.resolve` | `takeOurs`, `takeTheirs`, `combine`, `manual`, `adaptedDelta` |
| `unica.merge.apply` | `task`, `original` |
| `unica.merge.verify` | `localCheckpoint`, `synchronizedTask`, `synchronizedTaskAdapted`, `mainSandbox`, `mainIntegration` |
| `unica.repository.status` | absent |
| `unica.repository.update` | `routinePreview`, `routineApply`, `armPreview`, `armApply`, `prerequisitePreview`, `prerequisiteApply`, `cancellationPreview`, `cancellationApply` |
| `unica.repository.planLocks` | absent |
| `unica.repository.lock` | absent |
| `unica.repository.unlock` | `compensation`, `rollback`, `abandonment` |
| `unica.repository.commit` | `preview`, `apply` |
| `unica.repository.recover` | `recoverApply`, `recoverCancel` |

An omitted-`dryRun` preview and an explicit-`true` preview normalize to the same
logical `*Preview` selector. The cancellation receipt pair being absent
(`awaitingArm`) or present (`armed`) likewise does not create a caller-selectable
variant; current authorization state validates the same cancellation action.
Conversely, the supported-update replacement triple and synchronized-task
adaptation pair select their explicit logical variants because they bind a
different producer lineage. A single-variant selector rejects
`requestVariant`; a multi-variant selector requires exactly one value from its
own row. Table order is schema/snapshot order and no cross-tool alias is valid.
`IncomingToolName` is a 7-128-character ASCII string matching
`^unica\.[A-Za-z0-9][A-Za-z0-9._-]{0,121}$`, used only to report a rejected
unregistered/incompatible incoming name, never to dispatch or advertise a next
action. Optional request variants are permitted only by the named registered
tool's request union. Every `allowedNextActions` array is canonical and
duplicate-free: tool calls precede external instructions, tool calls follow the
tool/variant table order above, and external instructions follow their literal
order in the `NextAction` definition. The list is the exact safe action set
advertised by that error branch, not an exhaustive replacement for a fresh
status projection; for example a blocked abandonment may advertise status even
when status then proves an exact unlock call legal. This does not add a 22nd
branched lifecycle tool. Array order is canonical set serialization only, never
an instruction to execute every member in sequence; grammar names and prose use
table order, while a caller chooses one safe action and re-queries authoritative
state as that action requires.

`TaskErrorData` is a closed code/context tagged `oneOf`; every branch also has
`allowedNextActions: NextAction[]`:

- `bindingRejected { code: repositoryBindingMismatch |
  mainDiffersFromRepository, context: BindingErrorContext }`, where the closed
  context is `{ contextKind: binding, expectedBindingDigest,
  observedBindingDigest, originalFingerprint?, repositoryFingerprint? }`;
- `artifactRejected { code: artifactNotDistribution, context:
  ArtifactInputErrorContext }`. `ArtifactKindRole` is the closed four-way
  `oneOf` of `{ kind: configurationDistribution, role:
  baselineDistribution }`, `{ kind: configurationDistribution, role:
  refreshDistribution }`, `{ kind: configurationDistribution, role:
  supportRecoveryDistribution }`, or `{ kind: ordinaryConfiguration, role:
  ordinaryResult }`; no enum cross-product is accepted. The closed context is `{
  contextKind: artifactInput, artifactId, observedKind: ArtifactKind,
  observedRole: ArtifactRole, acceptedInputs: ArtifactKindRole[] }`;
- `lifecycleRejected { code: cleanupNotAllowed | taskAbandonmentNotSafe |
  recoveryPlanPending | taskPhaseMismatch | taskMutationBlocked, context:
  LifecycleErrorContext }`, closed `{ contextKind: lifecycle, phase: TaskPhase,
  allowedPhases: TaskPhase[], blockerCodes: StableErrorCode[],
  recoveryDigest?: Sha256, recoveryCancellationAllowed?: boolean }`;
- `operationRejected { code: operationReplayMismatch | operationInProgress,
  context: OperationErrorContext }`, closed `{ contextKind: operation,
  operationId, expectedInputDigest?: Sha256, observedInputDigest?: Sha256,
  activeOperationDigest?: Sha256 }`;
- `reservationRejected { code: targetReservationBusy |
  repositoryAccountReservationBusy, context: TargetReservationBusyContext |
  RepositoryAccountReservationBusyContext }`, where
  `TargetReservationBusyContext` is closed `{ contextKind: targetReservation,
  repositoryIdentityDigest: Sha256, originalInfobaseIdentityDigest: Sha256,
  reservationKeyDigest: Sha256, owner: ReservationOwnerRef }` and
  `RepositoryAccountReservationBusyContext` is closed `{ contextKind:
  repositoryAccountReservation, repositoryIdentityDigest: Sha256,
  normalizedUsernameDigest: Sha256, reservationKeyDigest: Sha256,
  owner: ReservationOwnerRef }`;
- `digestRejected { code: approvalDigestMismatch | changeReceiptStale, context:
  DigestErrorContext }`, closed `{ contextKind: digest, expectedDigest: Sha256,
  observedDigest: Sha256, producerId: UnicaId }`;
- `adaptationDecisionRejected { code: adaptationDecisionAlreadyRecorded,
  context: AdaptationDecisionConflictContext }`, closed `{ contextKind:
  adaptationDecision, verificationId: UnicaId, existingDecisionId: UnicaId,
  existingAdaptationDecisionDigest: Sha256 }`; the request does not carry a
  proposed decision digest, so this branch never fabricates unequal expected/
  observed decision digests;
- `conflictResolutionRejected { code: conflictResolutionNotAllowed, context:
  ConflictResolutionErrorContext }`, closed `{ contextKind:
  conflictResolution, sessionId, conflictId, conflictKind: ConflictKind,
  requestedResolution: ConflictResolution, allowedResolutions:
  ConflictResolution[] }`; the allowed list is non-empty, duplicate-free, in
  canonical resolution order, and excludes the requested value;
- `commitPolicyRejected { code: commitCommentPolicyMismatch, context:
  CommitCommentPolicyErrorContext }`, closed `{ contextKind:
  commitCommentPolicy, phase: mainValidated, expectedPolicyDigest: Sha256,
  observedPolicyDigest: Sha256, mismatchKinds:
  CommitCommentPolicyMismatchKind[] }`, where the closed mismatch enum is
  `templateChanged | taskMetadataChanged | renderEmpty | renderNotTaskBound`;
- `integrationSetRejected { code: integrationSetMismatch, context:
  IntegrationSetErrorContext }`, closed `{ contextKind: integrationSet,
  phase: locked | recoveryRequired, expectedLineageDigest: Sha256,
  observedLineageDigest: Sha256, mismatchKinds:
  IntegrationSetMismatchKind[], exitKind: unlock | recovery,
  lockSetId?: UnicaId, expectedLockSetDigest?: Sha256,
  recoveryDigest?: Sha256 }`, where the closed mismatch enum is
  `planSet | mergeSet | verificationSet | commitSet | lockSet`;
- `capabilityRejected { code: platformCapabilityUnproven |
  supportLayerAmbiguous | unsupportedChangeKind |
  exclusiveRepositoryUserRequired, context: CapabilityErrorContext }`, closed
  `{ contextKind: capability, capabilityKind: platform | supportLayer |
  repositoryUserExclusivity | changeSemantics,
  capabilityRowId?:
  CapabilityRowId, evidenceDigest?: Sha256 }`. Code/kind mapping is exact:
  `platformCapabilityUnproven` uses `platform`, `supportLayerAmbiguous` uses
  `supportLayer`, `exclusiveRepositoryUserRequired` uses
  `repositoryUserExclusivity`, and `unsupportedChangeKind` uses
  `changeSemantics`;
- `profileStateRejected { code: projectIdentityCollision |
  stateRootRelocationRequired | profileInvalid | secretUnavailable,
  context: ProfileStateErrorContext }`, closed `{ contextKind:
  profileState, projectId?, profile?, propertyPath?, expectedDigest?: Sha256,
  observedDigest?: Sha256 }`; or
- `stateCorruptRejected { code: stateCorrupt, context:
  StateCorruptErrorContext }`, where `StateCorruptStateRef` is the closed tagged
  `oneOf` of `workspace { stateRefKind: workspace,
  workspaceIdentityDigest: Sha256 }`, `startAttempt { stateRefKind:
  startAttempt, workspaceIdentityDigest: Sha256, taskId: TaskId, operationId:
  OperationId }`, `project { stateRefKind: project, projectId: ProjectId }`,
  `task { stateRefKind: task, projectId: ProjectId, taskId: TaskId, instanceId:
  UnicaId }`, or `taskOperation { stateRefKind: taskOperation, projectId:
  ProjectId, taskId: TaskId, instanceId: UnicaId, operationId: OperationId }`.
  The reference comes from the authenticated incoming request, storage key, and
  parent container, never from the corrupt bytes. Pre-project locators use
  `workspace`; start replay uses `startAttempt`; project locators/reservations
  use `project`; task journals/decisions/evidence/archive use `task`; operation
  records/receipts use `taskOperation`. The first two leaves remain
  representable before a valid `projectId` exists. `StateCorruptObservation` is
  the closed tagged `oneOf` of `exactBytes { observationKind: exactBytes,
  observedDigest: Sha256 }` or `unavailable { observationKind: unavailable,
  reason: missing | permissionDenied }`. The context therefore contains `{
  contextKind: stateCorrupt, stateRef, expectedDigest: Sha256, observation:
  StateCorruptObservation }`, not a flat optional digest bag. `exactBytes`
  requires its digest to differ from `expectedDigest`; `unavailable` has no
  observed/metadata/sentinel digest. Existing bytes/object state is retained
  untouched when present; a missing object has no fabricated bytes to retain;
  or
- `taskContextRejected { code: taskNotFound | taskWorkspaceContextInvalid |
  toolNotBranchedCompatible, context: TaskContextErrorContext }`, closed `{
  contextKind: taskContext, requestedTaskId,
  requestedToolName: IncomingToolName,
  workspaceMismatchKinds?: (projectMismatch | markerMissing | markerMismatch |
  leaseMissing | leaseInvalid)[], expectedProjectId?, observedProjectId?,
  expectedMarkerDigest?: Sha256, observedMarkerDigest?: Sha256 | null,
  expectedLeaseDigest?: Sha256, observedLeaseDigest?: Sha256 | null }`.
  Mismatch kinds are canonical/duplicate-free in the listed order;
  `markerMissing` and `markerMismatch` are mutually exclusive, as are
  `leaseMissing` and `leaseInvalid`.

For `commitPolicyRejected`, `expectedPolicyDigest` is the frozen start-time hash
of canonical `{ template, taskId, taskSummary, projectId, renderedComment,
nonEmpty: true, taskBound: true }`; `observedPolicyDigest` hashes the same closed
record from the current profile/task metadata and render result. The digests are
unequal and `mismatchKinds` is their exact semantic projection. For
`integrationSetRejected`, both lineage digests hash the same canonical ordered
record `{ planSetDigest, mergeSetDigest, verificationSetDigest,
commitSetDigest, lockSetDigest }`; they are unequal and `mismatchKinds` exactly
names every unequal member, with no net-digest/change-then-revert erasure.
For both branches, exact projection is a typed producer and replay-validation
invariant. `CommitCommentPolicyMismatchProofRecord` is the closed `{
proofKind: commitCommentPolicyMismatch, expected:
CommitCommentPolicyDigestRecord, observed: CommitCommentPolicyDigestRecord,
mismatchKinds }`; `IntegrationSetMismatchProofRecord` is the closed `{
proofKind: integrationSetMismatch, expected: IntegrationSetLineageDigestRecord,
observed: IntegrationSetLineageDigestRecord, mismatchKinds }`. Each list is
derived from its two records, never accepted as input, and each `proofDigest ==
sha256(canonical(proof-record))`. Before publishing the terminal rejection, the
producer atomically persists that exact content-addressed proof and the terminal
envelope's typed `evidence` contains only the closed `{ proofKind, proofDigest
}` reference. The context digests equal the canonical hashes of the proof's two
records and its kinds equal the proof projection. Replay resolves and rehashes
the immutable proof by that reference rather than consulting a possibly changed
current profile/render result. A missing, multiple, wrong-kind, wrong-digest, or
context-divergent proof makes the retained terminal a corrupt candidate before
replay. The wire context intentionally carries only opaque digests and the
derived list, so standalone JSON Schema/deserialization can enforce unequal
digests plus non-empty/canonical kinds but cannot reconstruct the projection.
No API accepts a caller-supplied projection as authoritative.

The generated schema splits those groups into literal-code branches with this
exhaustive presence/action grammar. `none` is `[]`; `statusOnly` is exactly one
`unica.branched.status` tool call; `recoveryResume` is status plus
`unica.repository.recover/recoverApply` and additionally `recoverCancel` iff
the context literal permits it; `adaptationRefresh` is status plus exactly
`unica.merge.verify/synchronizedTask`; `commitSafeExit` is status plus
`unica.branched.archive/abandonedPreview`, the selector for exact request
literals `{ outcome: abandoned, dryRun: true }`; the request also supplies its
required non-empty `reason`. Its stopped result must create the exact
restore/full-unlock recovery plan before `recoverApply` becomes legal.
`conflictReview` is status plus exactly `unica.merge.conflicts`; the context's
`sessionId` supplies the request selector and retrying `merge.resolve` is not
advertised until the caller has reviewed the current list.
`integrationSetExit` is status plus exactly `unica.repository.unlock/rollback`
for `exitKind: unlock`, or status plus exactly
`unica.repository.recover/recoverApply` for `exitKind: recovery`; and
`startAndStatus` is exactly the canonical selector array
`[unica.branched.start, unica.branched.status]`. These grammar descriptions name
set membership in canonical table order, not a prescribed execution sequence.

| Rejected code | Required/forbidden context | Exact allowed-action grammar |
| --- | --- | --- |
| `repositoryBindingMismatch` | binding digests required and unequal; fingerprints are diagnostics only and cannot select this code | `statusOnly` |
| `mainDiffersFromRepository` | binding digests required and equal; original/repository fingerprints required and unequal | `statusOnly` |
| `artifactNotDistribution` | all artifact fields required; accepted input tuples are non-empty/canonical and exclude the exact observed kind/role tuple. A distribution in the wrong role remains truthfully a distribution and is rejected by role; `configurationUpdate`/`invalidArtifact` cannot appear in accepted tuples | `statusOnly` |
| `cleanupNotAllowed`, `taskAbandonmentNotSafe` | `allowedPhases=[]`; blocker codes are non-empty; recovery fields are absent | `statusOnly` |
| `recoveryPlanPending` | `allowedPhases=[]`, `blockerCodes=[]`; recovery digest and cancellation-allowed literal are required | `recoveryResume` |
| `taskPhaseMismatch` | allowed phases are non-empty, canonical, duplicate-free, ordered by `TaskPhase`, and exclude the current `phase`; `blockerCodes=[]`; recovery fields are absent | `statusOnly` |
| `taskMutationBlocked` | `allowedPhases=[]`; blockers are non-empty; recovery digest/cancellation literal are absent. Any current recovery/unknown effect has higher-precedence `recoveryPlanPending` and cannot enter this row | `statusOnly` |
| `operationReplayMismatch` | operation ID and unequal expected/observed input digests required; active digest absent | `none` |
| `operationInProgress` | operation ID and active-operation digest required; input digests absent | `statusOnly` |
| `targetReservationBusy` | exact target/repository/key digests and closed owner reference required; no task state exists | `statusOnly` |
| `repositoryAccountReservationBusy` | exact repository/account/key digests and closed owner reference required; no task state exists | `statusOnly` |
| `approvalDigestMismatch`, `changeReceiptStale` | expected/observed digests and producer ID are required; digests are unequal; no producer selector or adaptation fields exist | `statusOnly` |
| `adaptationDecisionAlreadyRecorded` | verification ID, existing decision ID, and existing adaptation-decision digest are required; digest-comparison and generic producer fields are absent | `adaptationRefresh` |
| `conflictResolutionNotAllowed` | session/conflict/kind/requested resolution are required; allowed resolutions are non-empty, canonical, duplicate-free, equal the persisted conflict list, and exclude the requested value; digest/capability/lifecycle fields are forbidden | `conflictReview` |
| `commitCommentPolicyMismatch` | phase is exactly `mainValidated`; policy digests are required and unequal; mismatch kinds are non-empty/canonical and exactly project the frozen-template/task-metadata/render violation; producer/producer-ID, integration-set, lock-set, and recovery fields are forbidden | `commitSafeExit`; repeating commit or any producer is absent, and recovery is offered only after the abandoned-archive preview has published its exact plan |
| `integrationSetMismatch` | lineage digests are required and unequal; mismatch kinds are non-empty/canonical and exactly project the plan/merge/verification/commit/lock-set disagreement. `exitKind: unlock` requires phase `locked`, both lock-set fields, proven no original-merge intent, and no recovery digest. `exitKind: recovery` requires phase `recoveryRequired` plus the exact recovery digest and forbids both lock-set fields | `integrationSetExit`; adaptation refresh and commit retry are absent; unlock and recovery actions are mutually exclusive |
| `platformCapabilityUnproven`, `supportLayerAmbiguous`, `unsupportedChangeKind`, `exclusiveRepositoryUserRequired` | exact code-to-kind mapping from `CapabilityErrorContext` and evidence digest are required; row ID required only when a row was observed | `statusOnly` |
| `projectIdentityCollision`, `stateRootRelocationRequired` | project ID plus unequal expected/observed digests required | `statusOnly` |
| `stateCorrupt` | exact trusted `StateCorruptStateRef`, expected schema digest, and either unequal exact-byte observed digest or exact `missing`/`permissionDenied` unavailability required; no sentinel digest, path, or identity recovered from corrupt bytes | `statusOnly` |
| `profileInvalid`, `secretUnavailable` | profile and property path required; digest fields absent | `statusOnly` |
| `taskNotFound` | requested task/tool required; mismatch/project/marker/lease fields absent | `startAndStatus` |
| `taskWorkspaceContextInvalid` | non-empty mismatch kinds; project IDs required iff project mismatch; expected marker/lease digest plus observed digest-or-null required iff the matching kind is present, with null exactly for missing; unrelated fields absent | `none` |
| `toolNotBranchedCompatible` | requested task/tool required; mismatch/project/marker/lease fields absent | `none` |

Each code is legal in exactly one branch; contexts reject extra fields and all
diagnostics remain bounded/redacted. Tests instantiate every legal pair. The
wire schema rejects every structurally distinguishable cross-branch
code/context substitution, missing/extra field, or disallowed next-action
injection; validated deserialization additionally rejects the enumerated
equality, inequality, and membership predicates whose compared values are
present, while typed producer/replay validators reject exact-projection
violations that require authoritative preimages intentionally absent from the
wire context. Standard Draft 2020-12 cannot compare sibling values or recover
hash preimages. Tests freeze that explicit schema/deserializer-superset list so
no structural hole is mislabeled a relational limitation. They additionally reject either
`commitCommentPolicyMismatch` or `integrationSetMismatch` in
`DigestErrorContext`, any `TaskOperationSelector` field in a digest/adaptation
context, any `adaptationRefresh` action outside
`adaptationDecisionAlreadyRecorded`, commit-policy actions other than exact
`commitSafeExit`, an unlock integration exit with recovery fields/actions, and
a recovery integration exit with lock fields/unlock/cancel actions.
`StableErrorCode` and `RejectedCode`
contain the exact literals `commitCommentPolicyMismatch` and
`integrationSetMismatch` once each and reject aliases or renamed spellings.

The envelope's top-level `operationId` presence is fixed solely by the selected
descriptor policy: every `MutatingTaskResultEnvelope` carries the request's
exact value, while every `ReadOnlyTaskResultEnvelope` forbids the field. A
nested operation reference never changes that choice. `data` is one of the
exact tool-specific bound variants below, and `evidence` contains only its exact
bounded redacted identities, hashes, receipts, and diagnostics. `command`, raw
`stdout`, and raw `stderr` are always absent.

Stops/rejections after task lookup return their exact variant with the
unchanged/blocking/recovery task status. Schema/unknown-tool errors remain
application/MCP errors before a task result exists.

`branched.status` for a missing task is a successful read with
`status: "notCreated"` and `NotCreatedData { exists: false, startAllowed,
blockers[] }`. A failed `branched.start` preflight returns the `rejected` variant
with `status: "notCreated"`; its original-workspace-scoped start-attempt record preserves
replay without creating a task directory. Every other tool returns stable
`taskNotFound` when no task record exists.

Evidence-bearing domain stops are exhaustive:

| Producer | `stopCode` | Stopped `data` |
| --- | --- | --- |
| `delivery.verify` | `artifactKindMismatch` | `ArtifactClassificationStopData { artifactId, kind, expectedKind?, expectationMatched: false, sha256, probeId, supportIdentity?, currentEqualsVendor?, diagnosticsDigest, classificationDigest }`; there is no `verificationId` or selectable resume handle |
| any delivery/platform validation boundary | `platformWarningRejected` | `PlatformWarningStopData { producerTool: TaskOperationToolName, currentPhase, warnings: PlatformWarningEvidence[], diagnosticsDigest }`, where each closed warning is `{ warningCode, objectDisplay?, diagnostic }`, the list is non-empty/bounded/redacted, and no success handle or target effect is published |
| `repository.update(routine)` preview/apply before update intent | `repositoryStructureConfirmationUnproven` | `RepositoryStructureConfirmationStopData { mode: routine, structuralChanges: RepositoryPlannedChange[], requiredCapabilityKind: repositoryStructuralUpdate, observedCapabilityRowId?: CapabilityRowId, diagnosticsDigest }`; structural changes are non-empty, no update/lock remains, and phase/authorization is unchanged |
| `branched.archive` before retention release or `branched.cleanup` preview/apply | `unsafeTaskPath` | `PathGuardStopData { producerTool: unica.branched.archive | unica.branched.cleanup, guardKind: canonicalization | identityChanged | equal | ancestor | descendant | symlinkOrReparse, protectedBoundaryDigest, destructiveTarget?: OwnedTargetLocator, liveProviderPresent, diagnosticsDigest }`; no retention release, move, quarantine, or deletion occurs |
| `merge.prepare(supportedUpdate)` | `twiceChangedProperties`, `unresolvedReferences` | `MergeSessionData` with `sessionId`, immutable/evolving digests, conflict count, and optional resolution-workspace ID; conflict records come from `merge.conflicts`, and current handles from `branched.status` |
| `merge.prepare(resolvedReplay)` | `conflictDecisionsIncomplete`, `unboundResolutionChanges` | `ResolutionReplayStopData { sessionId, baseSessionDigest, decisionSetDigest, workspaceGenerationId, missingConflictIds[], unboundChangeReceiptIds[] }`; `missingConflictIds` is the canonical projection of conflicts in either `undecided` or `replacementPending` state. `unboundChangeReceiptIds` is the canonical sequence-ordered projection of current changed-receipt handles with `selectable: true`. No-change, consumed, superseded, and invalidated-generation receipts are excluded. Missing decisions take primary-code precedence when both lists are non-empty |
| `merge.prepare` | `vendorAncestryMismatch` | `MergePreparationStopData { mode, expectedAncestor, observedAncestor, checkpointId, recovery: RecoveryPlanStatus }`; status is `recoveryRequired`, with task-checkpoint restore/recreate planned to `localVerified` |
| `merge.prepare(mainIntegration)` | `relevantBaselineChanged` | `RelevantBaselineChangedStopData { comparisonId, expectedRelevantBaselineDigest, observedRelevantBaselineDigest, expectedHistoryCursor, observedHistoryCursor, historyPartition: RepositoryHistoryPartition, relevantHistoryEntries: RepositoryHistoryPartitionEntry[], mismatchKinds: [relevantBaselineChanged], differenceManifestId, differenceDigest }`; relevant entries are non-empty and may end in the same net baseline digest after change-then-revert; no lock/original effect, downstream synchronization evidence is invalidated, and status is `localVerified` |
| `merge.prepare(mainIntegration)` | `manualSupportRequired`, `vendorForbidsChanges`, `supportPreflightInconclusive` | `SupportPreflightStopData { preflight: SupportPreflightData, supportActionAuthorization?: SupportActionAuthorizationData, requiredExternalAction, allowedNextActions: NextAction[] }`; this is an outcome-tagged closed union. Manual requires an `awaitingArm` authorization plus `AcquireSupportRootInstruction` and the exact list status + arm-preview + cancellation-preview. Vendor-forbidden has no authorization, requires `VendorSupportDecisionInstruction`, and lists exactly status + `decideVendorRestriction`. Inconclusive has no authorization, requires `SupportEvidenceInstruction`, and lists exactly status + `provideSupportEvidence`. No cross-outcome action is legal; no main session/lock plan exists and status remains `synchronized` |
| `merge.prepare(mainIntegration)` | `mainPreparationMismatch` | `MainPreparationMismatchStopData { comparisonId, expectedRelevantBaselineDigest, observedRelevantBaselineDigest, expectedHistoryCursor, observedHistoryCursor, mismatchKinds[], differenceManifestId, differenceDigest }`; status is `validationFailed` and no main session is published |
| `merge.verify(mainSandbox)` / `repository.planLocks` / `repository.lock` before any lock effect | `relevantBaselineChanged` | `RelevantBaselineChangedStopData { comparisonId, expectedRelevantBaselineDigest, observedRelevantBaselineDigest, expectedHistoryCursor, observedHistoryCursor, historyPartition: RepositoryHistoryPartition, relevantHistoryEntries: RepositoryHistoryPartitionEntry[], mismatchKinds: [relevantBaselineChanged], differenceManifestId, differenceDigest }`; a non-empty relevant history list is sufficient even when the net digest is equal; stale Dn-and-later evidence is invalidated, no lock is acquired, and status is `localVerified` |
| `merge.verify(mainSandbox)` / `repository.planLocks` / `repository.lock` before any lock effect | `supportPreflightStale` | `SupportGateStaleStopData { supportGateId, expectedSupportGateDigest, observedSupportGateDigest, mismatchKinds: SupportGateMismatchKind[], expectedInputs: SupportGateInputDigests, observedInputs: SupportGateInputDigests, relevantBaselineDigest, expectedHistoryCursor, observedHistoryCursor, historyEvidence: SupportGateHistoryEvidence, originalCleanRefreshProof?: OriginalCleanRefreshProof, invalidatedSessionId? }`; this code requires `historyEvidence.gateObservedCursor == expectedHistoryCursor`, `classifiedThroughCursor == observedHistoryCursor`, an unchanged relevant baseline, an all-unrelated partition, and non-empty exact non-anchor mismatches. The clean-refresh proof is required exactly when mismatches contain `originalFingerprintChanged`; otherwise it is absent. Stale main evidence is invalidated, no lock is acquired, and status is `synchronized` |
| `repository.lock` after acquiring the root guard first | `relevantBaselineChanged` | `RelevantBaselineLockStopData { expectedRelevantBaselineDigest, observedRelevantBaselineDigest, expectedHistoryCursor, observedHistoryCursor, historyPartition: RepositoryHistoryPartition, relevantHistoryEntries: RepositoryHistoryPartitionEntry[], acquiredRootGuard, releasedRootGuard, compensationVerified: true }`; partition endpoints equal expected/observed cursors and relevant entries are non-empty even for an equal net digest. No other object lock is attempted; verified compensation returns `localVerified`, otherwise recovery |
| `repository.lock` after acquiring the root guard first | `supportPreflightStale` | `SupportGateStaleLockStopData { supportGateId, expectedSupportGateDigest, observedSupportGateDigest, mismatchKinds: SupportGateMismatchKind[], expectedInputs: SupportGateInputDigests, observedInputs: SupportGateInputDigests, expectedRelevantBaselineDigest, observedRelevantBaselineDigest, expectedHistoryCursor, observedHistoryCursor, historyEvidence: SupportGateHistoryEvidence, originalCleanRefreshProof?: OriginalCleanRefreshProof, acquiredRootGuard, releasedRootGuard, compensationVerified: true }`; history-evidence cursors equal the enclosing expected/observed cursors, the baseline is unchanged, and the partition is all-unrelated. The clean-refresh proof follows its exact mismatch presence rule. No other object lock is attempted; verified compensation returns `synchronized`, otherwise recovery |
| `merge.verify(localCheckpoint)` / `merge.verify(mainSandbox)` | `validationFailed` | `MergeVerificationData` with immutable diagnostics evidence |
| `merge.verify(synchronizedTask)` | `validationFailed`, `unexpectedDelta` | `MergeVerificationData` with immutable diagnostics/difference evidence |
| `merge.verify(mainIntegration)` | `mainMergeValidationFailed` | `MainMergeValidationStopData { verification: MergeVerificationData, recovery: RecoveryPlanStatus }`; status is `recoveryRequired` |
| `branched.archive(outcome="abandoned")` while preparing inverse cleanup authorization | `supportPreflightInconclusive` | `SupportCleanupEvidenceStopData` is a closed tagged `oneOf`: `classifiedCleanupEvidence { evidenceState: classifiedCleanupEvidence, originPhase, supportPrerequisiteReceiptIds[], currentSupportGraphDigest, requiredRestoreTransitions: SupportTransition[], evidenceGaps: SupportEvidenceGap[], requiredExternalAction: SupportEvidenceInstruction }` or `unclassifiedSupportGraph { evidenceState: unclassifiedSupportGraph, originPhase, supportPrerequisiteReceiptIds[], evidenceGaps: SupportEvidenceGap[], requiredExternalAction: SupportEvidenceInstruction }`. In the first, restore transitions/gaps are non-empty and gaps contain only authorization prerequisites; in the second, gaps are non-empty graph/layer-identity evidence and graph/transition fields are absent. The instruction has empty blockers and byte-identical gaps, no cleanup authorization/archive is created, and status stays at `originPhase` |
| `branched.archive(outcome="abandoned")` preview/apply with accepted task-only support transitions | `manualSupportCleanupRequired` | Preview returns `SupportCleanupPreviewStopData { stage: preview, proposal: SupportCleanupProposalData, previewDigest }`; the proposal's exact transition list is inverse-only, but no support action ID/authorization, external lease, or phase mutation exists. The distinct approved apply returns `SupportCleanupStopData { stage: apply, supportPrerequisiteReceiptIds[], currentSupportGraphDigest, originPhase, requiredRestoreTransitions: SupportTransition[], supportActionAuthorization: SupportActionAuthorizationData, requiredExternalAction: AcquireSupportRootInstruction, allowedNextActions: NextAction[] }`; authorization purpose is `abandonmentCleanup`, its exact transition list equals the approved proposal, its digest binds the ordered receipt chain/current graph/phase evidence, and it is published only as `awaitingArm` after journaled capability gates. The instruction forbids editing/commit before arming; the allowed list contains exactly status, `repository.update/armPreview`, and `repository.update/cancellationPreview`; apply variants are absent until their producer digests exist. No archive is created, status stays at `originPhase`, and exact inverse root-only reconciliation is required |
| `branched.archive(outcome="abandoned")` preview from `mainMerged`/`mainValidated` | `abandonmentRecoveryRequired` | `AbandonmentRecoveryStopData { recovery: RecoveryPlanStatus }`; it persists preview evidence only, leaves status unchanged, and requires `repository.recover` before archive can be previewed again |
| `repository.lock` | `repositoryLockConflict` | `RepositoryLockConflictData` |
| `repository.lock` | `repositoryLockRollbackFailed` | `RepositoryLockRollbackFailedData` |
| `repository.update(routine)` while acquiring the selective guard set | `repositoryLockConflict` | `RepositoryUpdateLockConflictData { mode: routine, planDigest, failedTarget: RepositoryTargetIdentity, failedTargetDisplay: RepositoryTargetDisplay, lockedBy: RepositoryOwnerIdentity | null, acquiredThenReleased: RepositoryUpdateLockTarget[], compensationVerified: true, requiredExternalAction: ReleaseRepositoryLocksInstruction }`; the released list is the exact reverse acquired prefix, no update runs, status is unchanged, and a fresh preview follows external release. Unverified compensation enters recovery |
| any `repository.update` apply after acquiring its selective guard set | `repositoryUpdatePlanStale` | `RepositoryUpdatePlanStaleData { mode, expectedPlanDigest, guardReceiptId, expectedTargets: RepositoryTargetState[], observedTargets: RepositoryTargetState[], observedHistoryPartition: RepositoryHistoryPartition, acquiredThenReleased: RepositoryUpdateLockTarget[], compensationVerified: true, authorizationOutcome?: unchanged, supportRootLockProof?: SupportRootLockProof }`; the list is the exact reverse acquired prefix of the approved plan's lock targets. The target map changed before update, no update runs, every temporary guard is released, and a fresh preview is required. For support reconciliation/cancellation, `authorizationOutcome: unchanged` and the root proof are required, its guard receipt equals the top-level receipt, and its release equals the same root lock window; both fields are absent for routine, whose receipt identifies its temporary selective guard. Unverified compensation enters recovery |
| `repository.update(supportPrerequisiteArm)` | `manualSupportRootLockRequired` | `SupportArmRootRequiredData { supportActionId, expectedSupportActionDigest, originPhase, observedRootLock: SupportRootLockObservation, requiredExternalAction: AcquireSupportRootInstruction | ReleaseRepositoryLocksInstruction }`; absent root returns the acquire instruction, a wrong proven owner returns release/coordination, no arming receipt or edit instruction exists, authorization remains `awaitingArm` |
| `repository.update(supportPrerequisiteArm)` | `supportPrerequisiteArmStale` | `SupportArmStaleData { stage: preview | applyRecheck, supportActionId, expectedSupportActionDigest, originPhase, evidence: SupportArmStaleEvidence, authorizationOutcome: unchanged, requiredNextMode: supportPrerequisiteCancellation, requiredExternalAction?: ReleaseRepositoryLocksInstruction }`. Neither stage may arm or cancel. The release instruction is required iff a proven manual root owner remains; after release, only the exact cancellation flow can publish its full terminal proof/receipt before fresh preflight |
| `repository.update(supportPrerequisiteArm)` | `supportPreflightInconclusive` | `SupportArmInconclusiveData { supportActionId, expectedSupportActionDigest, originPhase, historyPartition?: RepositoryHistoryPartition, evidenceGaps: SupportEvidenceGap[], requiredExternalAction: SupportEvidenceInstruction }`; complete contiguous history/root owner/support graph/recovery-handoff/original evidence is unavailable, no receipt/edit instruction exists, and authorization remains `awaitingArm` |
| `repository.update(supportPrerequisite)` | `manualSupportActionPending` | `SupportActionPendingData { supportActionId, expectedSupportActionDigest, armingReceiptId, expectedArmingReceiptDigest, priorSupportGateId, originPhase, observedRepositoryVersions[], partitionedRoutineChanges[], mismatchKinds: [noAuthorizedVersionObserved], requiredExternalAction: ManualSupportInstruction }`; the instruction is reconstructed byte-for-byte from the arming receipt, no support actor/delta fields exist, intervening routine versions may be classified but no effect occurs, authorization remains `armed`, and status stays at its bound `originPhase` |
| `repository.update(supportPrerequisite)` with no authorized version but proven disjoint external support | `supportPrerequisiteReconciliationRequired` | `SupportActionExternalAdvanceData { supportActionId, expectedSupportActionDigest, originPhase, historyPartition: RepositoryHistoryPartition, disjointExternalSupportChanges[], requiredNextMode: supportPrerequisiteCancellation }`; reconciliation cannot consume the authorization or pretend the human acted. The authorization remains armed, and its exact cancellation flow preserves/selectively applies the external root state, cancels the action, and returns to `relevantAdvancePhase` |
| `repository.update(supportPrerequisite)` | `manualSupportLocksRemain` | `SupportPrerequisiteLocksRemainData { supportActionId, expectedSupportActionDigest, priorSupportGateId, originPhase, observedRepositoryVersions[], observedActor: RepositoryActorIdentity, observedRootDeltaDigest, mismatchKinds[], remainingLocks[], ownedGuardRetained: false, supportRootLockProof?: SupportRootLockProof, requiredExternalAction: ReleaseRepositoryLocksInstruction }`; mismatch kinds contain only `rootLockRetained` and/or `manualActorLockInventoryChanged`, the version is otherwise exact, authorization stays armed, and status stays at `originPhase` |
| `repository.update(supportPrerequisite)` | `supportPreflightInconclusive` | `SupportPrerequisiteInconclusiveData { supportActionId, expectedSupportActionDigest, priorSupportGateId, originPhase, observedRepositoryVersions[], observedActor?: RepositoryActorIdentity, observedRootDeltaDigest?, mismatchKinds[], evidenceGaps: SupportEvidenceGap[], remainingLocks[], ownedGuardRetained: false, supportRootLockProof?: SupportRootLockProof, requiredExternalAction: SupportEvidenceInstruction }`; gaps are non-empty prerequisite-version/global records, equal the instruction gaps byte-for-byte, and their kind projection equals its missing-kind list. At least one exact attribution/classification field is unavailable, authorization remains armed, and status stays at `originPhase` |
| `repository.update(supportPrerequisite)` or `repository.update(supportPrerequisiteCancellation)` with authorization `state=armed` | `supportPrerequisiteConflict` | `SupportPrerequisiteConflictData { supportActionId, expectedSupportActionDigest, originPhase, historyPartition: RepositoryHistoryPartition, conflicts: SupportTransitionConflict[], supportRootLockProof?: SupportRootLockProof, requiredExternalAction: SupportConflictInstruction, recovery: RecoveryPlanStatus }`; conflicts are non-empty exact overlapping/unclassifiable external support deltas. No version is reversed or accepted; the authorization freezes and status enters `recoveryRequired` with `preserveExternalAndReauthorize`. An apply-attempt proof is required only if the root guard was acquired, with unchanged authorization and verified release. Recovery waits for a digest-bound external corrective sequence or another task's immutable ownership receipt, archives the full chain, preserves the resulting external baseline, cancels this action, and returns to its relevant-advance phase. An `awaitingArm` cancellation can never enter this row: complete root/support history is `preArmExternal`, incomplete history is inconclusive, and an unknown cancellation effect uses `preArmSupportCancellation` recovery |
| `repository.update(supportPrerequisite)` or `repository.update(supportPrerequisiteCancellation)` with authorization `state=armed` | `manualSupportPrerequisiteInvalid` | `SupportPrerequisiteInvalidData { supportActionId, supportActionDigest, originPhase, authorizedCancelledPhase, authorizedRelevantAdvancePhase, authorizedPostReconcilePhase, plannedResultPhase, recoveryDisposition, successfulIntegrationForbidden?: true, versionObservations: SupportPrerequisiteVersionObservation[], expectedOriginalFingerprint?, observedOriginalFingerprint?, originalDeltaDigest?, mismatchKinds: SupportPrerequisiteMismatchKind[], recovery: RecoveryPlanStatus }`; observations equal the recovery plan's complete cursor-range list. The three original fields are required together iff mismatches contain `originalNotClean` and absent otherwise; the observation list may be empty only for its versionless singleton case. That versionless case deterministically uses `restoreThenReauthorize`, selectively restores the original to the classified repository baseline, cancels the action, and plans the bound cancelled/relevant-safe phase; it is not immutable taint. The invalid-classification mismatch union plus that optional original mismatch equals the top-level list. Positively action-attributed invalid history freezes the authorization and enters `recoveryRequired`; `unauthorizedContentChanged` or `offSupportObserved` has precedence and requires `restoreThenAbandon`, the forbidden-success literal, and `plannedResultPhase: abandonmentReady`. Other action-attributed invalid history uses `restoreThenReauthorize` and `authorizedCancelledPhase`. Proven external wrong-mode/actor use selects `preserveExternalAndReauthorize`, never inverses that version, and plans `authorizedRelevantAdvancePhase`; overlap or unattributed provenance instead uses `supportPrerequisiteConflict`/inconclusive classification and cannot enter this row with an invented disposition. An `awaitingArm` cancellation can never enter this row or acquire a disposition. The nested plan fields must match byte-for-byte |
| `repository.update(supportPrerequisiteCancellation)` | `manualSupportLocksRemain` | `SupportCancellationBlockedData { supportActionId, expectedSupportActionDigest, priorSupportGateId, originPhase, observedRepositoryVersions[], partitionedRoutineChanges[], mismatchKinds[], remainingLocks[], ownedGuardRetained: false, supportRootLockProof?: SupportRootLockProof, requiredExternalAction: ReleaseRepositoryLocksInstruction }`; cancellation performs no effect, keeps authorization awaiting/armed at `originPhase`, and requires release before a fresh preview |
| `repository.update(supportPrerequisite)` or `repository.update(supportPrerequisiteCancellation)` apply | `manualSupportLocalChangesRemain` | `ManualSupportLocalChangesData` is a closed tagged `oneOf`: `separateWorkingInfobase { manualTargetMode: separateWorkingInfobase, supportActionId, expectedSupportActionDigest, originPhase, attemptedTerminalization: consume | cancel, workingInfobaseStop: ManualWorkingInfobaseStopEvidence, supportRootLockProof: SupportRootLockProof, terminalizationPerformed: false, requiredExternalAction: CleanManualWorkingInfobaseInstruction }` or `reservedOriginal { manualTargetMode: reservedOriginal, supportActionId, expectedSupportActionDigest, originPhase, attemptedTerminalization: consume | cancel, reservedOriginalLeaseStop: ReservedOriginalLeaseStopEvidence, supportRootLockProof: SupportRootLockProof, terminalizationPerformed: false, requiredExternalAction: CloseReservedOriginalDesignerInstruction }`. Only capability-proven busy/dirty outcomes are retryable; every acquired guard is released with `authorizationOutcome: unchanged`, no terminalization occurs. An unknown lease effect uses armed `supportPrerequisite` recovery or, for awaiting-action cancellation, the no-arming `preArmSupportCancellation` recovery |
| `repository.update(supportPrerequisiteCancellation)` | `supportPreflightInconclusive` | `SupportCancellationInconclusiveData { supportActionId, expectedSupportActionDigest, priorSupportGateId, originPhase, observedRepositoryVersions[], partitionedRoutineChanges[], expectedOriginalFingerprint, observedOriginalFingerprint?, expectedSupportGraphDigest, observedSupportGraphDigest?, evidenceGaps: SupportEvidenceGap[], remainingLocks[], supportRootLockProof?: SupportRootLockProof, requiredExternalAction: SupportEvidenceInstruction }`; gaps are non-empty prerequisite-version/global records, equal the instruction gaps byte-for-byte, and their kind projection equals its missing-kind list. Cancellation performs no effect, stays at `originPhase`, and cannot infer absence of the human action |
| `repository.recover` for frozen `supportPrerequisite` | `supportCorrectionPending` | `SupportCorrectionPendingData { recovery: RecoveryPlanStatus, newlyObservedVersionObservations: SupportPrerequisiteVersionObservation[], mismatchKinds: SupportPrerequisiteMismatchKind[], priorSupportRecoveryGuardProof?: SupportRecoveryGuardProof, requiredExternalAction: SupportCorrectiveInstruction }`; status remains `recoveryRequired`. The destination is still incomplete/wrong, the call durably appends every newly observed immutable version, recomputes the exact non-materialized finalization plan/digest and, in separate mode, keeps a `desired` working-IB closure plan without a future cursor/map. It performs no corrective repository effect. The prior-plan proof is required iff a materialized guard began before new drift and then released with unchanged authorization; it is immutable audit evidence and is absent from the recomputed plan's `latestSupportRecoveryGuardProof`. Repeated wrong corrections remain this stop |
| `repository.recover` after a valid correction/materialization changes the frozen plan | `supportRecoveryReapprovalRequired` | `SupportRecoveryReapprovalData { previousRecoveryDigest, recovery: RecoveryPlanStatus, reapprovalReasons: SupportRecoveryReapprovalReason[], materializedFinalizationPlan: SupportRecoveryFinalizationPlan, materializedManualWorkingInfobaseClosurePlan?: ManualWorkingInfobaseClosurePlan, priorSupportRecoveryGuardProof?: SupportRecoveryGuardProof }`; `previousRecoveryDigest` equals the approved request digest, the non-empty reasons identify exact appended history, ownership reclassification, and/or working-IB closure materialization, the top-level finalization plan is byte-identical to `recovery.supportRecoveryFinalizationPlan`, and its new `recoveryDigest` differs. The working-IB plan field is required exactly in separate mode, has literal state `materialized`, and is byte-identical to the nested recovery field; it is absent in reserved mode. Destination/history is now exact and no external action remains, but those materialized plans/digest have never been approved. No finalization effect occurs; status remains `recoveryRequired` and a fresh digest approval is mandatory. Any optional proof is the byte-identical released prior-plan attempt moved out of the recomputed plan's `latest` audit state |
| `repository.recover` for `repositoryCommit/observeOutcome` after a conclusive observation | `recoveryReapprovalRequired` | `RecoveryReapprovalData { previousRecoveryDigest, observationOutcome: committed | notCommitted, observationReceiptId, observationDigest, recovery: RecoveryPlanStatus }`; the new plan is the exact tagged committed release-only or not-committed restore/full-release branch, its digest differs, no branch effect has started, and explicit approval is required. Unknown observation remains `operationEffectUnknown` with the current observation plan rather than inventing a branch |
| `repository.recover` for `preArmSupportCancellation` when a fresh plan needs approval | `recoveryReapprovalRequired` | `PreArmCancellationRecoveryReapprovalData` is a closed `reapprovalCause`-tagged `oneOf`: `outcomeObserved { reapprovalCause: outcomeObserved, previousRecoveryDigest, effectObservation: PreArmCancellationEffectObservation, recovery: RecoveryPlanStatus }` or `finalizationRecheckChanged { reapprovalCause: finalizationRecheckChanged, previousRecoveryDigest, effectObservation: PreArmCancellationEffectObservation, recheckEvidence: PreArmCancellationFinalizationRecheckEvidence, compensatedAttempt: PreArmCancellationFinalizationAttemptAudit, recovery: RecoveryPlanStatus }`. The first new plan is exactly `preArmCancellationStage=finalize`, binds the conclusive original-operation observation and has begun no finalization effect. The second is legal only for `replannableBeforeUpdate` plus `recheckEvidence.outcome=replanRequired`: no update/cancellation occurred, the exact newly acquired guard prefix was released in reverse order, its immutable attempt audit is appended, and the new finalization attempt/digest differs. Unknown observation/compensation remains `operationEffectUnknown`; protected-update drift is a capability breach, not reapproval. No arming receipt or armed-support disposition is legal |
| `repository.recover` for `preArmSupportCancellation/finalize` with a conclusive acquisition blocker | `preArmCancellationRecoveryBlocked` | `PreArmCancellationRecoveryBlockedData` is a closed `blockerKind`-tagged `oneOf`: `rootGuardConflict { blockerKind: rootGuardConflict, previousRecoveryDigest, compensatedAttempt: PreArmCancellationFinalizationAttemptAudit, knownBlocker: PreArmCancellationKnownBlocker, recovery: RecoveryPlanStatus, failedTarget: RepositoryTargetIdentity, failedTargetDisplay: RepositoryTargetDisplay, lockedBy: RepositoryOwnerIdentity | null, requiredExternalAction: ReleaseRepositoryLocksInstruction }` or `modeLeaseUnavailable { blockerKind: modeLeaseUnavailable, previousRecoveryDigest, manualTargetMode: ManualSupportTargetMode, compensatedAttempt: PreArmCancellationFinalizationAttemptAudit, knownBlocker: PreArmCancellationKnownBlocker, recovery: RecoveryPlanStatus, workingInfobaseStop?: ManualWorkingInfobaseStopEvidence, reservedOriginalLeaseStop?: ReservedOriginalLeaseStopEvidence, requiredExternalAction: CleanManualWorkingInfobaseInstruction | CloseReservedOriginalDesignerInstruction }`. `knownBlocker` has the same discriminator and is byte-identical to the branch's previous digest, compensated-attempt audit digest, blocker evidence, and instruction; it is also byte-identical to `recovery.preArmCancellationKnownBlocker`, while `compensatedAttempt` is the fresh plan's last full prior-attempt audit. Root conflict has empty forward/compensation receipt lists and no guard to release. For mode blockage, reserved mode and `leaseBusy` have no mode acquisition; their forward list contains root acquisition only when the plan started root-released, and compensation contains the exact root release (including release of an inherited prior-operation root). Separate-mode `leaseAcquiredDirty` instead has a mode-acquisition receipt in the forward list and mode-release then root-release receipts in compensation; the stop evidence's lease receipt IDs match those effect receipts exactly. The stop/instruction fields follow the exclusive mode presence rule. Both append the compensated attempt and durable blocker to a fresh plan that starts both guards released, keep the authorization frozen without arming, and require blocker resolution plus explicit approval; status returns the same blocker/instruction after response loss. Unknown acquire/release effects keep effect-unknown recovery instead |
| `repository.recover` for frozen external-support conflict | `supportConflictResolutionPending` | `SupportConflictResolutionPendingData { recovery: RecoveryPlanStatus, newlyObservedVersionObservations: SupportPrerequisiteVersionObservation[], conflicts: SupportTransitionConflict[], priorSupportRecoveryGuardProof?: SupportRecoveryGuardProof, requiredExternalAction: SupportConflictInstruction }`; the prior-plan proof exists iff finalization locking began before the newly observed conflict, is a stopped/blocked variant with unchanged authorization and verified release, and is absent from the recomputed plan's `latest` state. Status remains `recoveryRequired`, no automatic reversal/terminalization occurs, and the full chain plus recomputed digest is persisted until a valid external corrective sequence or immutable ownership receipt proves the disposition-bound external baseline |
| `repository.recover` before publishing a corrective instruction when recovery distribution/handoff evidence is unavailable | `supportPreflightInconclusive` | `SupportRecoveryEvidencePendingData { recovery: RecoveryPlanStatus, evidenceGaps: SupportEvidenceGap[], requiredExternalAction: SupportEvidenceInstruction }`; gaps are non-empty recovery-artifact/handoff/retention/readability evidence, equal the instruction with empty blockers, no corrective instruction/finalization effect occurs, and status remains `recoveryRequired`. A retention-lease breach also invalidates the capability row and cannot be waved through |
| `repository.recover` for frozen `supportPrerequisite` | `supportRecoveryBlockedByLock` | `SupportRecoveryLockBlockedData { recovery: RecoveryPlanStatus, supportRecoveryGuardProof: SupportRecoveryGuardProof, failedTarget: RepositoryTargetIdentity, failedTargetDisplay: RepositoryTargetDisplay, lockedBy: RepositoryOwnerIdentity | null, requiredExternalAction: ReleaseRepositoryLocksInstruction }`; the proof is `blockedBeforeRoot` or `blockedAfterPartial`, has unchanged authorization, matches the next failed target/owner, and proves compensation of the acquired prefix. Status remains `recoveryRequired`; unverified compensation uses the unknown-effect recovery path |
| `repository.recover` for frozen `supportPrerequisite` in `separateWorkingInfobase` | `manualSupportLocalChangesRemain` | `FrozenSupportLocalChangesData { recovery: RecoveryPlanStatus, workingInfobaseStop: ManualWorkingInfobaseStopEvidence, supportRecoveryGuardProof: SupportRecoveryGuardProof, terminalizationPerformed: false, requiredExternalAction: CleanManualWorkingInfobaseInstruction }`; the guard proof is `stoppedAfterCompleteGuard`, the recovery plan remains current in `recoveryRequired`, every acquired guard is proven released with unchanged authorization, and a new approved recovery attempt is required after the human closes/cleans the IB |
| `repository.recover` for frozen `supportPrerequisite` in `reservedOriginal` | `manualSupportLocalChangesRemain` | `FrozenReservedOriginalClosureData { recovery: RecoveryPlanStatus, reservedOriginalLeaseStop: ReservedOriginalLeaseStopEvidence, supportRecoveryGuardProof: SupportRecoveryGuardProof, terminalizationPerformed: false, requiredExternalAction: CloseReservedOriginalDesignerInstruction }`; the proof is `stoppedAfterCompleteGuard`, all repository guards are released with unchanged authorization, and no finalization occurs until a newly approved attempt acquires the exclusive original lease |
| `repository.update(routine)` preview with a current deferred-advance handle in any phase | `supportPreflightInconclusive` | `DeferredAdvanceInconclusiveData { deferredRepositoryAdvance: DeferredRepositoryAdvance, currentPhase, expectedHistoryCursor: RepositoryHistoryCursor, observedHistoryCursor?: RepositoryHistoryCursor, historyPartition?: RepositoryHistoryPartition, evidenceGaps: SupportEvidenceGap[], requiredExternalAction: SupportEvidenceInstruction }`; the expected cursor equals the handle's `fromCursor`; gaps are non-empty `repositoryHistoryEvidence` records, use an exact first version only when known, and equal the instruction projection. No update or handle consumption occurs, phase is unchanged, and only a later routine preview with complete contiguous classification can proceed |
| `repository.update(routine)` preview from `abandonmentReady` without a deferred-advance handle | `supportPreflightInconclusive` | `AbandonmentRefreshInconclusiveData { beforeAnchor, observedRepositoryVersions[], plannedChanges[], missingEvidenceKinds: SupportMissingEvidenceKind[], requiredExternalAction: SupportEvidenceInstruction }`; the typed list equals the instruction projection, no update occurs, and status remains `abandonmentReady` |
| `merge.apply(target="original")` pre-effect guard | `relevantBaselineChanged` | `RelevantMergeApplyStopData { sessionId, expectedRelevantBaselineDigest, observedRelevantBaselineDigest, expectedHistoryCursor, observedHistoryCursor, historyPartition: RepositoryHistoryPartition, relevantHistoryEntries: RepositoryHistoryPartitionEntry[], lockSetId, expectedLockSetDigest, requiredNextTool: repository.unlock }`; endpoints match, relevant entries are non-empty even for net-zero digest, no task merge starts, and status is `staleRelevantBaseline` until exact unlock |
| `merge.apply(target="original")` pre-effect guard | `supportPreflightStale` | `SupportMergeApplyStaleData { sessionId, supportGateId, expectedSupportGateDigest, observedSupportGateDigest, supportMismatchKinds: SupportGateMismatchKind[], expectedSupportInputs: SupportGateInputDigests, observedSupportInputs: SupportGateInputDigests, relevantBaselineDigest, expectedHistoryCursor, observedHistoryCursor, historyEvidence: SupportGateHistoryEvidence, originalCleanRefreshProof?: OriginalCleanRefreshProof, lockSetId, expectedLockSetDigest, requiredNextTool: repository.unlock }`; history-evidence cursors equal expected/observed, the partition is all-unrelated, and the clean-refresh proof follows the fingerprint-mismatch presence rule. No task merge starts; status is `staleSupportPreflight` until exact unlock. An unowned/unclassified original delta uses recovery instead |
| `merge.apply(target="original")` pre-effect guard | `additionalLocksRequired` | `AdditionalLocksMergeApplyStopData { sessionId, supportGateId, lockSetId, expectedLockSetDigest, additionalLockEntries[], requiredNextTool: repository.unlock }`; additional entries are non-empty, history/support-stale fields are absent, no task merge starts, and status is `lockPlanExpansionRequired` until exact unlock |
| `repository.commit` preview or immediate pre-effect guard | `postMergeLineageChanged` | `PostMergeLineageStopData { mergeReceiptId, verificationId, supportGateId, expectedConsumedSupportGateDigest, observedSupportGateState, expectedAuthorizedPostMergeFingerprint, observedOriginalFingerprint, expectedHistoryCursor, observedHistoryCursor, expectedReferenceClosureDigest, observedReferenceClosureDigest, historyTailPartition: RepositoryHistoryPartition, conflictingEntries: RepositoryHistoryPartitionEntry[], integrationSetId, lockSetId, recovery: RecoveryPlanStatus }`; conflicting entries are non-empty and identify the relevant/referrer/support cause; no commit starts, status is `recoveryRequired`, and the exact restore-plus-full-unlock plan is mandatory |
| `repository.commit` | `repositoryCommitFailed` | `RepositoryMutationObservationData { operationId, observedRepositoryAnchor?, observedObjects[], observedLocks[], recovery: RecoveryPlanStatus }`; status is `commitBlocked` |
| `repository.commit` | `repositoryCommitAmbiguous`, `repositoryUnlockUnverified` | `RepositoryMutationObservationData { operationId, observedRepositoryAnchor?, observedObjects[], observedLocks[], recovery: RecoveryPlanStatus }`; status is `committedUnverified` |
| `repository.unlock` | `repositoryUnlockUnverified` | `RepositoryMutationObservationData { operationId, observedRepositoryAnchor?, observedObjects[], observedLocks[], recovery: RecoveryPlanStatus }`; status is `recoveryRequired` |
| target-effect-free `readOnly` platform inspection with proven process-tree termination | `operationTimedOut` | `ReadOnlyTimeoutData { operationClass, observedTermination, temporaryEvidenceDiscarded: true, resumePhase }`; no operation ID exists and status is unchanged |
| `contained` operation with proven process termination and owned-area postcondition | `operationTimedOut` | `ContainedTimeoutData { operationId, operationClass, observedTermination, observedOwnedState, retainedEvidenceIds[], resumePhase }`; status remains the pre-operation safe phase |
| authoritative effect, or any effect whose postcondition is not proven | `operationTimedOut`, `operationEffectUnknown`, `rollbackUnproven` | `RecoveryRequiredData { recovery: RecoveryPlanStatus }`; status is `recoveryRequired` unless the more specific commit-ambiguity state applies |

Every stable code not listed in this matrix is a `rejected` result with its
named `TaskErrorData.context`; it cannot carry a tool-success/stop data variant.

Supported-update precedence is exact: if twice-changed properties and
unresolved references coexist, the same `MergeSessionData` retains every
conflict, `stopCode` is `twiceChangedProperties`, and `errors[0].code` equals
that stop code. `unresolvedReferences` is primary only when no twice-changed
conflict remains; `MergeSessionData` retains both conditions without a
secondary error entry.

Support-prerequisite code selection is deterministic. For reconciliation, the
singleton `noAuthorizedVersionObserved` maps to `manualSupportActionPending`
only when every intervening entry is routine. A proven disjoint external-support
entry with no authorized version instead maps to
`supportPrerequisiteReconciliationRequired` and the exact cancellation mode.
For either reconciliation or otherwise-eligible no-version cancellation, a
non-empty subset of `rootLockRetained` and
`manualActorLockInventoryChanged` maps to `manualSupportLocksRemain` once the
available history/content/actor evidence is complete; the semantic trigger is
an unclosed manual-support lock window, not the existence of a valid version.
During any separate-mode terminalization apply, a capability-proven busy lease
rejection or a cleanly released exclusive lease that observed local state maps
to `manualSupportLocalChangesRemain` and takes no terminalization effect.
During reserved-mode terminalization, only a capability-proven exclusive-lease
busy/closed-session rejection maps to that stop; acquired inspection must either
produce the terminal proof or the versionless `originalNotClean` path. Unknown
lease acquisition, inspection, or release is support-prerequisite recovery.
`versionUnattributed` or other missing capability-proven
actor/working-IB/lock evidence maps to `supportPreflightInconclusive`. Any of
`reservedAccountUsed`, `reservedOriginalUsed`,
`multipleAuthorizedVersions`, `targetModeMismatch`, `unauthorizedContentChanged`, `unexpectedSupportTransition`,
`supportLayerChanged`, `offSupportObserved`, or `originalNotClean` takes
precedence as `manualSupportPrerequisiteInvalid`. A history-backed violation
freezes the authorization for recovery only when the violating version is
positively attributed to this action. The explicit exception is a versionless
singleton `originalNotClean`: it has no invented version observation, freezes
the armed authorization, records the exact expected/observed original
fingerprints, and deterministically selects `restoreThenReauthorize`. Such
action-attributed `unauthorizedContentChanged` or
`offSupportObserved` has disposition precedence over every other mismatch and
forces `restoreThenAbandon`. Other action-attributed violations select
`restoreThenReauthorize`. A wrong actor/target proven external selects
`preserveExternalAndReauthorize` and is never inversed; an unattributed or
overlapping mixed version instead uses `supportPrerequisiteConflict` (or
pre-freeze inconclusive evidence), never an arbitrary disposition. The two `reserved*Used` kinds are violations only in
`separateWorkingInfobase`; using a different actor/infobase in
`reservedOriginal` is `targetModeMismatch`. These immutable violations never
remain ordinarily retryable.

For the lock/inconclusive stop variants above, `supportRootLockProof` is absent
on preview. It is required iff an apply successfully acquired the root guard,
must have `authorizationOutcome: unchanged`, and proves release. A clean known
rejection of the first acquisition carries the exact root/owner in
`remainingLocks` and no acquisition proof. Any unverified release uses the
recovery-required lock-effect outcome instead of these retryable stops.

Every recovery-bearing stop that duplicates `requiredExternalAction` at the
top level (`SupportPrerequisiteConflictData`, `SupportCorrectionPendingData`,
`SupportConflictResolutionPendingData`, `SupportRecoveryLockBlockedData`, or
`FrozenSupportLocalChangesData`/`FrozenReservedOriginalClosureData`) requires
it byte-for-byte equal to
`recovery.requiredExternalAction`. Conflict lists equal the nested conflict
instruction; a lock stop's failed target/owner equals the current guard proof
and lock-release instruction; working-IB identity, closure-plan digest,
capability, and expected fingerprint equal the stop evidence, current nested
closure plan, and cleanup instruction. Reserved-original identity/capability
likewise equal the stopped lease evidence, frozen authorization, and closure
instruction. Every top-level newly observed list is
the exact contiguous suffix appended to `recovery.supportVersionObservations`
and covered by the new `recoveryDigest`; a reapproval reason similarly binds
its exact appended or reclassified observations. No user-facing instruction may
be spliced from another recovery plan.

## Execution Policies

`ExecutionPolicy` is the closed enum `readOnly`, `localJournaled`, `contained`,
`preparedJournaledEffect`, `journaledEffect`, or
`previewedJournaledEffect`.
`DurableExecutionPolicy` is the closed enum `localJournaled`, `contained`,
`preparedJournaledEffect`, `journaledEffect`, or
`previewedJournaledEffect`; it deliberately excludes `readOnly` and is the only
policy type legal in durable operation storage.
Policy is selected from the request's closed discriminator before dispatch. A
tool descriptor may declare one default only when all variants share it;
otherwise it publishes an exhaustive variant-to-policy map, as
`repository.recover` does for apply/cancellation and `repository.update` does
for local-only arming versus external-effect modes.

| Policy | Meaning |
| --- | --- |
| `readOnly` | Creates no `OperationRecord`, `OperationLease`, start-attempt record, receipt, durable preview/evidence handle, or task/status mutation; any returned operation reference denotes a pre-existing mutating record. It has no `operationId` or `dryRun`; temporary process output is ephemeral and discarded after bounded termination |
| `localJournaled` | Requires `operationId`; journals only owned local state/work-root creation or an atomic task decision; no external repository/infobase effect and no fake `dryRun` |
| `contained` | Requires `operationId`; records the operation and mutates only an owned probe/sandbox/evidence area; external sandbox calls use the `intentWritten`/`effectUnknown`/terminal record states, with any observed receipt as an fsynced barrier inside `effectUnknown`; no fake `dryRun` |
| `preparedJournaledEffect` | Requires `operationId` plus an exact prepared/session/status digest approval; journals intent before an authoritative infobase effect and verifies/reconciles its postcondition; no `dryRun` |
| `journaledEffect` | Requires `operationId` plus an exact guard digest; writes intent before external effect and verifies postcondition; no `dryRun` |
| `previewedJournaledEffect` | `dryRun: true` is target-effect-free, persists only its operation/preview evidence, and returns a preview-only data variant plus exact effect digest; a distinct applied operation binds that digest, journals intent, performs the owned/external effect, and verifies or reconciles postconditions |

Unknown-effect replay is never an execution policy. It returns
`operationEffectUnknown` until `repository.recover` reconciles the recorded
effect.

`OperationLease` is the closed `{ ownerInstanceId: UnicaId, generation,
acquiredAt, heartbeatAt, expiresAt, heartbeatDigest, leaseDigest }`; generation
is a positive monotonic integer and timestamps are normalized UTC instants.
`heartbeatDigest == sha256(canonical({ ownerInstanceId, generation,
heartbeatAt, expiresAt }))` with exactly those four members; `acquiredAt` is
instead bound by the outer `leaseDigest ==
sha256(canonical(lease-without-leaseDigest))`.

`OperationScope` is the closed tagged `oneOf` of `startAttempt { scopeKind:
startAttempt, workspaceIdentityDigest: Sha256, taskId: TaskId }` or `task {
scopeKind: task, projectId: ProjectId, taskId: TaskId, instanceId: UnicaId }`.
The start digest is issued by the canonical original-workspace identity
boundary after resolving `cwd` and before profile/project validation; it is
never a hash of the caller's spelling of `cwd`, a persisted path, or a
caller-supplied identity. Aliases of the same canonical workspace yield the
same digest. The start-attempt storage key is the exact tuple
`(workspaceIdentityDigest, taskId, operationId)`. A task-scoped loader instead
requires every scope member to equal the authoritative parent task record and
coordination locator. Thus moving a schema-valid record between workspaces,
projects, tasks, or instances is detectable before replay.

The storage foundation is the closed generic
`OperationRecord<TerminalEnvelope> { operationId, scope: OperationScope,
operation: TaskOperationSelector, policy: DurableExecutionPolicy, canonicalInputDigest,
registeredAt, operationLease?: OperationLease,
lastOperationLeaseDigest?: Sha256, state: registered | intentWritten |
effectUnknown | terminal, terminalEnvelopeDigest?: Sha256,
terminalEnvelope?: TerminalEnvelope, recoveryDigest?: Sha256 }`. Task 11 may
instantiate this shape only with a private closed test terminal type; it creates
no production alias, final terminal catalog, normalized storage snapshot, or
schema digest before Tasks 12-16 define the real result variants.
`operation` is the sole durable producer discriminator. Its typed selector
already contains the exact tool and physical request variant, so sibling
`toolName` or `requestVariant` fields are forbidden; `canonicalInputDigest` is
one-way evidence and cannot reconstruct a missing variant. The descriptor
derives `operation` from the validated closed request before registration; it is
never an additional caller-selected request field. Replay must derive the same
selector as well as the same canonical input digest.
The `startAttempt` scope is legal exactly for `unica.branched.start`; every
other durable selector requires `task` scope. A successful start terminal also
requires its `StartData.projectId`/`instanceId`, newly created task record, and
coordination locator to agree exactly; an early failed start remains replayable
without inventing either identity.

Task 16 creates the sole production binding
`CurrentOperationRecord = OperationRecord<MutatingTaskResultEnvelope>`. The
durable field `terminalEnvelope` MUST therefore be a
`MutatingTaskResultEnvelope`; `ReadOnlyTaskResultEnvelope` and the wider
`TaskResultEnvelope` union are never legal terminal type arguments. Every
mutating request creates or reads only `CurrentOperationRecord`. Its closed
schema/typed loader binds the exact `operation` selector to its legal durable
policy and matching mutating terminal-envelope producer. For a terminal record,
it also requires `terminalEnvelope.operationId == OperationRecord.operationId`;
`terminalEnvelope.taskId == OperationRecord.scope.taskId`; the envelope can
never be replayed under another operation or task key. It rejects every
selector for a physical `readOnly` variant, including a read-only branch of a
mixed-policy tool, not merely the impossible `policy: readOnly` literal. Task 17
is the first task that normalizes this fully bound schema, computes its expected
digest, and commits the catalog/storage snapshots.

The terminal fields are required together exactly for `terminal`; the recovery
digest is required exactly for `effectUnknown`.
`operationLease` is required for `registered`/`intentWritten` and absent for
`effectUnknown`/`terminal`; `lastOperationLeaseDigest` is absent while a current
lease exists and otherwise records its final generation for audit.
Storage schema validation runs before lease acquisition, status projection, or
replay classification. A persisted or legacy operation record with
`policy: readOnly`, a `readOnly` physical `operation` selector, an
operation/policy mismatch, an operation/policy/terminal-envelope mismatch, or a
record/envelope operation-ID or task-ID mismatch, scope/container mismatch, or
scope/selector mismatch is invalid and deterministically returns rejected
`stateCorrupt`: `expectedDigest` is the current operation-record schema digest.
When bytes were read, `observation` is `exactBytes` with their SHA-256 and the
record is retained for offline repair; when the expected object is absent or
cannot be read because of permissions, `observation` is the exact `unavailable`
leaf and no sentinel/metadata digest is fabricated. No CAS, lease, worker, receipt, dispatch, replay,
or external effect is allowed. Migration may never coerce that record to a
mutating policy or silently delete it.
The canonical input is exactly `OperationInputDigestRecord`; it includes the
tool name, selected policy, common fields, exact tagged request variant, and
every approval/guard digest, with only `operationId` excluded from its own hash.
Replay dispatch occurs before current phase, pending-recovery, or descendant-
evidence gates:

- the same operation ID and byte-equivalent canonical input in `terminal`
  returns the original immutable closed envelope byte-for-byte, including a
  terminal recover-apply or recover-cancellation response;
- the same ID with a different input returns `operationReplayMismatch` and
  performs no check/effect;
- `registered`/`intentWritten` with a live owner returns
  `operationInProgress` plus the typed active-operation status and never spawns
  a second worker;
- an orphaned `registered` record is capability-proven to have no effect intent;
  replay of the same canonical request may CAS-acquire its operation lease and
  resume ordinary dispatch from that record, while status merely reports it;
- an orphaned `intentWritten` or `effectUnknown` record is observed and
  reconciled through its exact recovery plan; it is never replayed blindly;
  and
- only a never-seen operation ID or the same-input CAS-reacquired no-intent
  `registered` record reaches ordinary phase/effect dispatch.

“Live owner” means the current lease generation is unexpired and its heartbeat
is valid; “orphaned” requires expired lease plus capability-proven absence of
that generation's worker. Reacquisition is one CAS over the complete record and
increments generation, so wall-clock observation alone never authorizes a
second worker.

`terminalEnvelopeDigest == sha256(canonical(terminalEnvelope))`. The complete
payload is stored inline or behind a content-addressed durable result record;
`recentOperations` is only a bounded projection and is never the replay source.
The durable state machine is exactly `registered -> intentWritten ->
effectUnknown -> terminal`. “Observed” is a policy-specific fsynced
evidence/receipt barrier while the common record remains `effectUnknown`, not a
fifth `OperationRecord.state`; a crash after that barrier but before
terminal-envelope persistence reconstructs the terminal result from the
receipt and never repeats the effect. Schema/crash tests cover response loss at
registration, intent, that observed barrier, terminal persistence, and after
terminal persistence but before send.
Coordination tests race two resumers against one expired generation and prove
exactly one atomic generation increment; an old owner heartbeat/intent carrying
the prior generation is rejected and cannot regain effect authority.

For `previewedJournaledEffect`, a preview and its apply are distinct requests
with distinct operation IDs. The apply binds the immutable preview digest;
reusing the preview's operation ID with `dryRun: false` is an input mismatch.
Every `previewedJournaledEffect` request is a strict tagged union: preview omits
`dryRun` or supplies the literal `true` and has no approval field; apply requires
the literal `dryRun: false` plus the tool-specific approved digest. The schemas
do not model a generic required boolean.

## Task Lifecycle Tools

### `unica.branched.start` — `localJournaled`

Request:

```text
taskId: TaskId (required)
operationId: OperationId (required)
cwd: workspace selector (required)
profile: non-empty local profile name (required)
taskSummary: non-empty immutable task summary (required)
```

Start validates config schema, secret availability (not values), exact
platform and retention-provider capability rows plus their tracked evidence,
target identity, and before task state/work-root creation atomically acquires
the authoritative target reservation `(repository identity, original-infobase
identity)` and repository-account reservation `(repository identity, normalized
integration username)`. The coordinator fails closed and uses fenced,
idempotent observe/renew/release receipts: persistent reservations survive
process-lease loss, an unknown receipt effect blocks replay and a second start,
and local mutexes remain process-local guards only. Missing/unproven required
cross-host exclusion returns `platformCapabilityUnproven` before task creation.
The validated platform topology row records `originalEndpointReachability` and
`repositoryEndpointReachability`, each exactly `hostConfined` or `multiHost`,
plus the required `crossHostReservationExclusion` case. An unproven
network-mounted file endpoint is `multiHost`. If either endpoint is
`multiHost`, that case proves a linearizable shared coordinator reachable by
every Unica host/account that can access either endpoint and one atomic
reservation over both keys; a per-user file or mutex cannot satisfy it.
Its retained evidence contains two independent, freshly isolated races:
`targetKeyExclusion` races the same canonical target under different normalized
integration accounts and admits exactly one start, with the loser rejected as
`targetReservationBusy`; `accountKeyExclusion` races different canonical
targets under the same normalized integration account and admits exactly one
start, with the loser rejected as `repositoryAccountReservationBusy`. The
non-contended key differs in each race, so an account-only or target-only
mechanism cannot pass both subcases.
It then validates leases, state/work paths, cleanup/comment policy, required manual target mode/conditional actor-history identity and
mode-specific service inspection/exclusive-lease endpoint and capability,
the crash-stable pre-arm guard capability,
pre-existing unresolved tasks before it creates the durable journal
and owned instance. It has no repository/infobase effect and no fabricated
preview. Before task creation it writes an original-workspace-scoped start-attempt record,
so failed preflight is replayable without a disposable directory. `data` is
`StartData { instanceId, projectId,
profile, originalInfobaseKind, repositoryTransport, capabilityRowId,
preArmCancellationGuardCapabilityId: CapabilityRowId,
retentionProviderCapabilityRowIds: CapabilityRowId[],
manualTargetMode: ManualSupportTargetMode,
manualActorUsername,
reservedOriginalLeaseCapabilityId?: CapabilityRowId,
manualWorkingInfobaseIdentity?: ManualWorkingInfobaseIdentity,
manualWorkingInfobaseInspectionCapabilityId?: CapabilityRowId,
workRootLocator: OwnedTargetLocator, commitCommentPreview }`. The working-IB
identity and inspection capability are required only for
`separateWorkingInfobase`; in reserved mode both are absent, the reserved lease
capability is required, and the manual actor equals the reserved integration
username. The retention-provider row list is canonical, non-empty exactly when
the profile declares recovery-distribution sources, and each ID resolves to the
validated tracked retention manifest described above. The reserved capability
is absent in separate mode. Both inspection endpoints and service
secret references remain profile-only and are never returned.
`preArmCancellationGuardCapabilityId` names a real fixture proving that the
configuration-root repository capture and the selected mode lease survive
worker, connector, and client-process death until an explicit receipt-bound
release, and that a restarted service can observe their owner/generation before
acting. If either guard can disappear implicitly, the manual-support workflow is
disabled with `platformCapabilityUnproven` before any authorization is
published; the state machine never assumes a held guard merely from a dead
worker's journal.

### `unica.branched.status` — `readOnly`

Request: no fields beyond common `cwd`/`taskId`. Existing-task `data` is
`TaskStatusData { exists: true, instanceId, phase,
taskWorkspaceId?, activeOperation?: ActiveOperationStatus,
pendingDecisions: PendingDecisionStatus[], anchors: TaskAnchorStatus[],
ownedLocks: OwnedLockStatus[], validationGates: ValidationGateStatus[],
artifactHashes: ArtifactHashStatus[], resumeHandles[], recentOperations[],
recovery?, latestDeferredAdvanceConsumption?:
DeferredRepositoryAdvanceConsumptionReceipt, archive?: TaskArchiveStatus,
cleanupReceipt?: CleanupReceipt,
cleanupEligibility: CleanupEligibilityStatus }`.
The named status records are closed:

- `ActiveOperationStatus { operationId, operation: TaskOperationSelector,
  policy: DurableExecutionPolicy, state: registered | intentWritten | effectUnknown,
  canonicalInputDigest, registeredAt, operationLease?: OperationLease,
  ownerState: live | orphaned,
  recoveryDigest?: Sha256 }`; the recovery digest is required exactly for
  `effectUnknown`; the lease is required for the first two states and absent
  for effect-unknown, and every field is a projection of `OperationRecord`;
- `PendingDecisionStatus { decisionKind: mergeConflict | adaptation,
  producerId: UnicaId, decisionIds: UnicaId[],
  replacementPendingDecisionIds: UnicaId[], remainingCount,
  decisionSetDigest }`; for `mergeConflict`, `decisionIds` contains only the
  canonical current decision head for each decided conflict, `remainingCount`
  is the exact number of conflicts without a current head, and
  `replacementPendingDecisionIds` is the canonical projection of the distinct
  historical heads in `replacementPending` state. `decisionSetDigest` hashes
  every conflict's closed undecided/current/replacement-pending state. For
  `adaptation`, the replacement-pending list is the literal empty list.
  Historical superseded/replaced decisions never enter `decisionIds` or the
  replay projection;
- `TaskAnchorStatus { anchorKind: repositoryCursor | taskFingerprint |
  originalFingerprint | vendorFingerprint, cursor?: RepositoryHistoryCursor,
  fingerprint?: Sha256, anchorDigest }`; cursor exists only for the repository
  variant and fingerprint only for the other variants;
- `OwnedLockStatus { target: RepositoryTargetIdentity, owner:
  RepositoryOwnerIdentity, acquisitionReceiptId, lockDigest }`;
- `ValidationGateStatus { gateKind: checkpoint | support | mainMerge |
  integrationSet, gateId: UnicaId, gateDigest, state: current | consumed }`;
- `ArtifactHashStatus { artifactId, role: ArtifactRole, kind: ArtifactKind,
  sha256 }`;
- `TaskArchiveStatus { archiveId, outcome: success | abandoned, sha256,
  retainedLineageDigest }`; and
- `CleanupEligibilityStatus { eligible, archiveId?: UnicaId,
  blockerCodes: StableErrorCode[], eligibilityDigest }`; `archiveId` is present
  exactly when an archive exists and `eligible: true` requires an archive plus
  no blocker.

Every array above is canonical and duplicate-free by its identity key.
`activeOperation` is absent when no journaled operation is non-terminal;
`archive` is present exactly in `archivedSuccess`, `archivedAbandoned`,
`cleanedSuccess`, and `cleanedAbandoned`, while cleanup retains that immutable
archive record. `cleanupReceipt` is required exactly in the two `cleaned*`
phases and absent before terminal cleanup. These
records are status projections only and cannot be supplied as mutation input.
`taskWorkspaceId` appears from successful deployment until cleanup and lets a
new client resume typed tools without learning a path. `resumeHandles` is a
closed `handleKind`-tagged union containing only current non-invalidated
records. Every branch below includes `handleKind` with the literal branch name
(`artifact`, `workspace`, `mergeResolutionWorkspace`, `checkpoint`,
`comparison`, `supportPreflight`, `supportActionAuthorization`,
`supportPrerequisite`, `supportCancellation`, `supportRecovery`,
`deferredRepositoryAdvance`, `mergeSession`, `decision`,
`resolutionChangeReceipt`, `verification`, `mergeApply`, `lockPlan`, `lockSet`,
`preview`, `recovery`, or `archive`); this outer discriminator is distinct from
any branch-local `kind` or `decisionKind` field:

- `artifact { artifactId, role, kind, sha256, verificationId? }`;
- `workspace { taskWorkspaceId }` or `mergeResolutionWorkspace {
  sessionId, workspaceId, baseSessionDigest }`;
- `checkpoint { checkpointId, scope (local|synchronized), sourceFingerprint }`;
- `comparison { comparisonId, scope, leftAnchor, rightAnchor, deltaDigest }`;
- `supportPreflight { supportGateId, outcome, candidateSetId,
  candidateSetDigest, supportGraphDigest, observedHistoryCursor,
  relevantBaselineDigest,
  ordinaryResultArtifactId, comparisonId, supportGateDigest,
  supportRecoveryDistributionSetDigest,
  historyEvidence: SupportGateHistoryEvidence, state,
  consumedByMergeReceiptId?, authorizedPostMergeFingerprint? }`; `state` is
  `current` or `consumedByOriginalMerge`. Only the latest non-invalidated
  `ready/current` observation can be supplied to lock planning or original
  apply. The two optional fields are required together only after successful
  original apply; that historical handle can be supplied only to the bound
  post-merge verification/commit lineage. A non-`ready` handle cannot be
  supplied to lock planning;
- `supportActionAuthorization { supportActionId, supportActionDigest,
  purpose, supportGateId, supportGateDigest, candidateSetDigest,
  expectedBeforeHistoryCursor, expectedRelevantBaselineDigest,
  armingRequired: true,
  authorizedTransitions: SupportTransition[], authorizedTransitionsDigest,
  supportRecoveryDistributions: SupportRecoveryDistributionEvidence[],
  supportRecoveryDistributionSetDigest,
  manualTargetMode, reservedIntegrationUsername,
  reservedOriginalIdentityDigest, reservedOriginalLeaseCapabilityId?,
  expectedOriginalFingerprint,
  manualActorUsername, manualActorLockBaselineDigest?,
  manualWorkingInfobaseIdentity?: ManualWorkingInfobaseIdentity,
  manualWorkingInfobaseBaseline?: ManualWorkingInfobaseBaseline,
  armingReceipt?: SupportActionArmingReceipt,
  originPhase, cancelledPhase, relevantAdvancePhase, postReconcilePhase,
  phaseEvidenceDigest, state: awaitingArm | armed | frozenForRecovery,
  freezeKind?: armedAction | preArmCancellationEffect }`; this is required for a manual
  prerequisite and survives intervening anchor advances only for exact
  arming/reconciliation/cancellation. `awaitingArm` permits only arm,
  cancellation, and status; `armed` permits reconciliation, cancellation, and
  status. Receipt and `freezeKind` presence follow the authorization rule above. Frozen
  state is read-only recovery context and is legal only with its exact
  `repository.recover` plan. Neither is valid for a main session, verification,
  plan, or lock;
- `supportPrerequisite { receiptId, priorSupportActionId, priorSupportGateId,
  purpose, armingReceiptId, armingReceiptDigest, repositoryVersion,
  repositoryActor, manualTargetMode,
  authorizedTransitionsDigest, rootDeltaDigest, rootLockProofDigest,
  historyFromCursor, historyThroughCursor, historyPartitionDigest,
  selectiveUpdateProofDigest, postReleaseObservedHistoryCursor,
  postApplyHistoryPartitionDigest,
  manualActorLockInventoryProofDigest?,
  reservedOriginalTerminalizationProofDigest?,
  manualWorkingInfobaseClosureProofDigest?, deferredRepositoryAdvanceDigest?,
  resultingPhase }`; the actor and reserved-original terminalization proof
  digests are required exactly for `reservedOriginal`, while the closure proof is
  required exactly for `separateWorkingInfobase`. This audit handle survives descendant-
  evidence invalidation and is retained in the archive;
- `supportCancellation { receiptId, receiptDigest, priorSupportActionId, purpose, reason,
  armingReceiptId?, armingReceiptDigest?, manualTargetMode, rootLockProofDigest,
  historyFromCursor, historyThroughCursor, historyPartitionDigest,
  preservedExternalSupportDigest, selectiveUpdateProofDigest,
  postReleaseObservedHistoryCursor, postApplyHistoryPartitionDigest,
  manualActorLockInventoryProofDigest?,
  reservedOriginalTerminalizationProofDigest?,
  manualWorkingInfobaseClosureProofDigest?, deferredRepositoryAdvanceDigest?,
  preArmCancellationEffectObservation?: PreArmCancellationEffectObservation,
  preArmCancellationFinalizationPlan?:
  PreArmCancellationFinalizationPlan,
  preArmCancellationFinalizationPlanDigest?,
  preArmCancellationReceiptPlanDigest?,
  preArmCancellationFinalizationRecheckEvidence?:
  PreArmCancellationFinalizationRecheckEvidence,
  preArmCancellationCompletedProgress?:
  PreArmCancellationFinalizationAttemptProgress,
  preArmCancellationFinalizationAttemptAuditDigest?,
  preArmRecoveryReceiptId?, preArmRecoveryReceiptDigest?,
  recoveryReceiptDigest?,
  resultingPhase }`; the arming fields are present together iff the cancelled
  action had armed. The mode-specific
  proof digests follow the same reserved/separate exclusive presence rule. This immutable handle exists after every completed
  cancellation, including an empty selective target set, survives descendant
  invalidation, and is retained in the archive.
  `receiptDigest` is the immutable cancellation receipt digest and
  `preservedExternalSupportDigest == sha256(canonical(the ordered
  externalSupport plus preArmExternal observations preserved by
  cancellation))`; the pre-arm subset is legal only when arming fields are
  absent. All named pre-arm recovery fields are required together exactly when
  cancellation completed through `preArmSupportCancellation` recovery, are
  absent for ordinary cancellation, and reproduce the terminal recovery data
  field-by-field; `priorSupportActionId == supportActionId`, `receiptId` and
  `receiptDigest` equal its cancellation receipt pair, and the embedded observation's digest is the terminal effect-
  observation digest. The full plan's digest and nested receipt-plan digest
  equal the two compact digest fields. The completed-progress branch has the
  same finalization attempt ID, contains every realized finalization receipt,
  reproduces the final recheck evidence, and its `attemptAuditDigest` equals
  `preArmCancellationFinalizationAttemptAuditDigest`. Thus status after response
  loss retains the complete immutable plan, prior compensated-attempt audits,
  completed receipt progress, and terminal observation without pretending it
  was armed;
- `supportRecovery { receiptId, priorSupportActionId, armingReceiptId,
  armingReceiptDigest, disposition,
  manualTargetMode,
  successfulIntegrationForbidden?: true, historyFromCursor,
  historyThroughCursor, postReleaseObservedHistoryCursor,
  postReleaseHistoryPartitionDigest, supportVersionObservationDigest,
  supportRecoveryFinalizationPlanDigest,
  supportRecoveryGuardProofDigest,
  reservedOriginalTerminalizationProofDigest?,
  manualWorkingInfobaseClosureProofDigest?, deferredRepositoryAdvanceDigest?,
  resultingPhase }`; the forbidden-success
  literal is required exactly for `restoreThenAbandon`; the closure-proof digest
  is required exactly for `separateWorkingInfobase` and absent for
  `reservedOriginal`, while `reservedOriginalTerminalizationProofDigest` is
  required exactly for `reservedOriginal` and absent for
  `separateWorkingInfobase`. This immutable handle
  survives plan clearance and is retained in every later archive;
  for this handle and both preceding support terminal handles,
  `historyFromCursor` equals the prior authorization's
  `expectedBeforeHistoryCursor`, `historyThroughCursor` and
  `historyPartitionDigest` (where present) equal the exact reconciled/frozen
  partition endpoint/digest, and the arming receipt prefix is unchanged. The
  post-release cursor/partition begins at that through cursor and no receipt may
  omit an earlier version;
- `deferredRepositoryAdvance { advance: DeferredRepositoryAdvance }`; it is
  present exactly when the latest terminal support receipt has the matching
  deferred digest and no later verified
  `DeferredRepositoryAdvanceConsumptionReceipt` names that terminal receipt
  and observation digest. The immutable terminal receipt keeps its historical
  deferred digest after consumption; status instead exposes the matching
  consumption receipt in `latestDeferredAdvanceConsumption` and omits the
  current handle. It gates every authoritative call except status and
  `repository.update(mode="routine")`. For `classified`/`unclassified`, that
  routine partition must begin with the recorded exact immediate successor;
  for `coverageUnknown`, it must first capability-prove complete history from
  `fromCursor` and discover the immediate successor without inventing one. A
  preview only binds/reports this resolution and leaves the handle current;
  only a verified approved routine apply atomically consumes it;
- `mergeSession { sessionId, mode, checkpointId, incomingDistributionId?,
  comparisonId, supportGateId?, supportGateDigest?, baseSessionDigest,
  supportGateHistoryEvidenceDigest?, decisionSetDigest,
  resolvedSessionDigest?, conflictCount }`; `comparisonId`
  is the exact comparison produced/consumed by
  that session, including `mainIntegration`. `incomingDistributionId` is
  required for supported-update sessions (including resolved-replay results)
  and absent for main integration; both support-gate fields are required only
  for main integration; the history-evidence digest is required with them;
- `decision` is the closed `decisionKind`-tagged `oneOf` of `mergeConflict {
  decisionId, decisionKind: mergeConflict, sessionId, baseSessionDigest,
  conflictId, resolution, rationaleDigest, changeReceiptDigest?,
  replacesDecisionId?, decisionDigest,
  revisedDecisionSetDigest, current, supersededByChangeReceiptId?,
  replacedByDecisionId? }` or `adaptation { decisionId, decisionKind:
  adaptation, verificationId, adaptationDecisionDigest }`. Merge-conflict handles forbid
  `verificationId`; adaptation handles forbid `sessionId` and
  merge-decision fields. The merge branch's producer fields through
  `revisedDecisionSetDigest` are byte-identical to its immutable decision
  record; the final three fields are a derived lifecycle projection. A current
  head has `current: true` and neither lifecycle ID. A historical head has
  `current: false` and at least one lifecycle ID: the change-receipt ID is
  present exactly when that receipt's `supersededDecisionIds` names it;
  `replacedByDecisionId` is present exactly when a later decision's
  `replacesDecisionId` names it. Both may eventually be present, but neither
  immutable decision body nor any consumed change receipt is rewritten. There
  is at most one current merge decision per conflict;
- `resolutionChangeReceipt { changeReceiptId, affectedTarget:
  BranchedAffectedTarget, afterSha256, changeReceiptDigest,
  supersededChangeReceiptIds[], supersededDecisionIds[],
  pendingReplacementDecisionId?,
  decisionSetDigestBefore, revisedDecisionSetDigest,
  phaseTransition: MergeResolutionPhaseTransition,
  baseSessionDigest, workspaceGenerationId, receiptSequence, consumed,
  selectable, supersededByReceiptId? }`; the target and receipt-lineage fields
  are the byte-identical singleton projection of a
  `MergeResolutionChangeReceipt`; no `MergeResolutionNoChangeReceipt` produces
  this handle. `supersededByReceiptId` is present exactly when that later
  same-generation/same-target changed receipt names this receipt in its
  `supersededChangeReceiptIds`; it is immutable, points to a strictly greater
  sequence, and is absent for consumed receipts. `selectable` is true exactly
  when the session and workspace generation are live, `consumed` is false, and
  `supersededByReceiptId` is absent. Consumption or generation invalidation
  makes it false. A superseded handle remains auditable with `consumed: false`
  but cannot be selected by `merge.resolve`; receipts for other targets retain
  their prior selectable state;
- `verification { verificationId, scope, sessionId?, checkpointId?, outcome,
  verificationDigest, canonicalDeltaDigest, differenceManifestId?, differenceDigest?,
  adaptationDecisionId?, mergeReceiptId?, integrationSetDigest?,
  supportGateHistoryEvidenceDigest? }`; for
  `synchronizedTask+unexpected`, both difference fields are required, and other
  optional fields have the same exact scope/outcome presence rules as
  `MergeVerificationData`. `sessionId` is required for every scope except
  `localCheckpoint`; `checkpointId` is required for a valid local checkpoint or
  equivalent/adapted synchronized-task result and absent otherwise;
- `mergeApply { mergeReceiptId, target, sessionId, resolvedSessionDigest,
  resultFingerprint,
  repositoryHistoryCursor?,
  rollbackCheckpointId?, sourcePublicationId?, sourceFingerprint?,
  taskInfobaseFingerprint?, integrationSetId?, integrationSetDigest?,
  supportGateHistoryEvidenceDigest? }`;
- `lockPlan { planId, planDigest, mergeSessionId, resolvedSessionDigest,
  supportGateId, supportGateDigest, supportGateHistoryEvidenceDigest,
  verificationId, verificationDigest, integrationSetId,
  integrationSetDigest }`;
- `lockSet { lockSetId, lockSetDigest, planId, planDigest, integrationSetId,
  integrationSetDigest, supportGateHistoryEvidenceDigest }`;
- `preview { toolName, previewOperationId, previewDigest, request }`, where
  `request` is the closed union `archive { outcome, reason? }`, `cleanup {
  archiveId }`, `deliveryCreate { role, inspectionDigest }`, `deliveryDeploy {
  distributionId }`, `repositoryUpdateRoutine { mode: routine,
  expectedStatusDigest }`, `repositoryUpdateSupportPrerequisite { mode:
  supportPrerequisite, expectedStatusDigest, supportActionId,
  expectedSupportActionDigest, expectedArmingReceiptId,
  expectedArmingReceiptDigest }`, or
  `repositoryCommit { integrationSetId, expectedIntegrationSetDigest,
  lockSetId, expectedLockSetDigest, verificationId,
  expectedVerificationDigest, mergeReceiptId, supportGateId,
  expectedSupportGateDigest, expectedSupportGateHistoryEvidenceDigest,
  expectedAuthorizedPostMergeFingerprint }`, or
  `supportPrerequisiteCancellation {
  mode: supportPrerequisiteCancellation, expectedStatusDigest, supportActionId,
  expectedSupportActionDigest, expectedArmingReceiptId?,
  expectedArmingReceiptDigest?, reason }`; the two optional arming fields are
  required together exactly when the approved cancellation preview observed an
  `armed` authorization and absent for `awaitingArm`;
- `recovery { priorOperationId, recoveryDigest }`;
- `archive { archiveId, sha256, outcome }`.

The top-level `latestDeferredAdvanceConsumption` is required exactly when the
most recent terminal support receipt carried a deferred observation that has
been consumed; it is absent while that observation is current or when no such
observation exists. Archive retains both the immutable terminal digest and any
matching consumption receipt. It is not a `resumeHandles` variant.

The read-only `supportPrerequisiteArm` preview is deliberately absent from this
durable preview union: it has no operation ID or state effect and is safely
repeated after response loss. Only its local-journaled apply can create the
durable arming receipt exposed by `supportActionAuthorization`.

`PreArmCancellationEffectKind` is the closed enum `rootGuardAcquire |
modeLeaseAcquire | selectiveOriginalUpdate | authorizationCancellation |
modeLeaseRelease | rootGuardRelease | recoveryFinalization`.
`PreArmCancellationEffectReceipt` is the closed `{
receiptKind: preArmCancellationEffect, receiptId, effectKind:
PreArmCancellationEffectKind, effectIntentDigest, producerActionId,
producerActionDigest, terminalObservationDigests: Sha256[], receiptDigest }`.
Its observation list is non-empty, unique, and preserves the enclosing action's
exact `expectedObservations` order; `canonical` here means the canonical action
projection in that order, never lexical sorting by digest. `receiptDigest ==
sha256(canonical(receipt-without-receiptDigest))`.

`PreArmCancellationReceiptRef` is the closed `source`-tagged `oneOf` of
`priorOperation { source: priorOperation, receipt:
PreArmCancellationEffectReceipt }`, `finalizationPlan { source:
finalizationPlan, receiptId, effectKind: PreArmCancellationEffectKind,
effectIntentDigest }`.
For `priorOperation`, the receipt's `effectIntentDigest` is the original interrupted operation's
pre-effect hash of `{ effectKind, priorOperationId, supportActionId,
expectedSupportActionDigest, approvedCancellationDigest, manualTargetMode,
selectiveUpdatePlanDigest, expectedPostconditionDigest }`; it cannot contain a
future recovery observation. For `finalizationPlan`, `effectIntentDigest` instead hashes canonical
`{ effectKind, finalizationAttemptId, supportActionId,
expectedSupportActionDigest, approvedCancellationDigest,
effectObservationDigest, manualTargetMode, selectiveUpdatePlanDigest,
expectedPostconditionDigest }`. Both formulas deliberately exclude every
receipt ID/digest, action/finalization/receipt-plan digest, and runtime result.
This breaks every plan/action/receipt hash cycle. `priorOperation` embeds the
byte-identical immutable journal receipt. `finalizationPlan` binds only the
future receipt ID, effect kind, and independent intent; its receipt digest exists only
after the effect has a terminal observation. Its sole legal realization is a
`PreArmCancellationEffectReceipt` in current-attempt progress with the same
receipt ID, effect kind, and intent digest plus the actual producer action/
terminal observations/receipt digest. The immutable plan ref is never rewritten;
`priorOperation` can never become a finalization-plan ref. Negative fixtures attempt every cyclic/
substituted source/kind/attempt/intent/ID/digest splice. A ref's effect kind is fixed by its enclosing
field: root/mode acquisition, update, cancellation, mode/root release, or local
recovery finalization respectively.

`PreArmCancellationSelectiveUpdateEffect` is the closed pre-release record `{
updateEffectReceipt: PreArmCancellationReceiptRef,
selectiveUpdatePlanDigest,
rootGuardAcquisitionReceipt: PreArmCancellationReceiptRef,
modeLeaseAcquisitionReceipt: PreArmCancellationReceiptRef,
plannedTargets: RepositoryTargetState[],
appliedTargets: RepositoryTargetState[], appliedTargetRevisionMapDigest,
beforeOriginalTargetFingerprintDigest,
verifiedOriginalTargetFingerprintDigest, observedBeforeCursor,
observedEffectCursor, selectiveObjectsCapabilityId,
structuralConfirmationRequired, structuralConfirmationUsed,
effectDigest }`. Both target lists equal the approved plan byte-for-byte and
each other; their map digest uses the ordinary selective-update formula. The
two acquisition receipts are `source=priorOperation` and equal the enclosing progress
prefix. Structural flags
and capability IDs equal the plan and use only the narrowly derived repository-
update confirmation. The original target fingerprint and observed effect cursor
prove the update postcondition while both guards are still held. This is not a
`SelectiveRepositoryUpdateProof`: it contains no release list, release claim,
post-release cursor, or authorization terminalization. `effectDigest ==
sha256(canonical(effect-without-effectDigest))`.

`PreArmCancellationSelectiveUpdateAlreadyExactEvidence` is the closed
no-effect record `{ selectiveUpdatePlanDigest,
rootGuardAcquisitionReceipt: PreArmCancellationReceiptRef,
modeLeaseAcquisitionReceipt: PreArmCancellationReceiptRef,
plannedTargets: RepositoryTargetState[], expectedTargetRevisionMapDigest,
beforeOriginalTargetFingerprintMapDigest,
verifiedOriginalTargetFingerprintDigest, observedBeforeCursor,
selectiveObjectsCapabilityId, structuralConfirmationRequired,
structuralCapabilityRowId?: CapabilityRowId,
structuralConfirmationUsed: false, evidenceDigest }`. Its planned target list is
non-empty and equals the approved plan byte-for-byte; both acquisition receipts
are `source=priorOperation`, and the before/verified fingerprint digests prove every
selected target already at its exact expected revision while both guards were
held. The structural capability row is present iff the approved plan requires
it and equals that plan, although no confirmation is consumed because no
invocation occurs. No repository-update invocation or update-effect receipt exists.
`evidenceDigest == sha256(canonical(evidence-without-evidenceDigest))`.

`PreArmCancellationUpdateProgress` is the closed tagged `oneOf` of
`notRequired { updateState: notRequired }` or `applied { updateState: applied,
selectiveUpdateEffect: PreArmCancellationSelectiveUpdateEffect }`, or
`alreadyExact { updateState: alreadyExact, alreadyExactEvidence:
PreArmCancellationSelectiveUpdateAlreadyExactEvidence }`.
`notRequired` is legal only when the approved cancellation selective target set
is empty; `alreadyExact` is legal only for a non-empty target set and reproduces
the exact guarded no-effect evidence; `applied` reproduces its exact pre-release
effect and receipt. Only terminal finalization may combine that update/no-effect
evidence with authorization and verified release into the full
`SelectiveRepositoryUpdateProof`.

`PreArmCancellationEffectProgress` is the following closed `stage`-tagged
`oneOf`; fields not named by a branch are forbidden:

- `noGuard { stage: noGuard }`;
- `rootHeldBeforeLease { stage: rootHeldBeforeLease,
  rootGuardAcquisitionReceipt: PreArmCancellationReceiptRef }`;
- `rootReleasedBeforeLease { stage: rootReleasedBeforeLease,
  rootGuardAcquisitionReceipt: PreArmCancellationReceiptRef,
  rootGuardReleaseReceipt: PreArmCancellationReceiptRef }`;
- `guardsHeldBeforeUpdate { stage: guardsHeldBeforeUpdate,
  rootGuardAcquisitionReceipt: PreArmCancellationReceiptRef,
  modeLeaseAcquisitionReceipt: PreArmCancellationReceiptRef }`;
- `modeReleasedBeforeUpdateRootHeld { stage:
  modeReleasedBeforeUpdateRootHeld,
  rootGuardAcquisitionReceipt: PreArmCancellationReceiptRef,
  modeLeaseAcquisitionReceipt: PreArmCancellationReceiptRef,
  modeLeaseReleaseReceipt: PreArmCancellationReceiptRef }`;
- `guardsReleasedBeforeUpdate { stage: guardsReleasedBeforeUpdate,
  rootGuardAcquisitionReceipt: PreArmCancellationReceiptRef,
  modeLeaseAcquisitionReceipt: PreArmCancellationReceiptRef,
  modeLeaseReleaseReceipt: PreArmCancellationReceiptRef,
  rootGuardReleaseReceipt: PreArmCancellationReceiptRef }`;
- `updateReadyGuardsHeld { stage: updateReadyGuardsHeld,
  rootGuardAcquisitionReceipt: PreArmCancellationReceiptRef,
  modeLeaseAcquisitionReceipt: PreArmCancellationReceiptRef,
  updateProgress: PreArmCancellationUpdateProgress }`;
- `cancellationPersistedGuardsHeld { stage:
  cancellationPersistedGuardsHeld,
  rootGuardAcquisitionReceipt: PreArmCancellationReceiptRef,
  modeLeaseAcquisitionReceipt: PreArmCancellationReceiptRef,
  updateProgress: PreArmCancellationUpdateProgress,
  cancellationPersistenceReceipt: PreArmCancellationReceiptRef }`;
- `cancellationPersistedModeReleased { stage:
  cancellationPersistedModeReleased,
  rootGuardAcquisitionReceipt: PreArmCancellationReceiptRef,
  modeLeaseAcquisitionReceipt: PreArmCancellationReceiptRef,
  updateProgress: PreArmCancellationUpdateProgress,
  cancellationPersistenceReceipt: PreArmCancellationReceiptRef,
  modeLeaseReleaseReceipt: PreArmCancellationReceiptRef }`; or
- `cancellationPersistedReleased { stage: cancellationPersistedReleased,
  rootGuardAcquisitionReceipt: PreArmCancellationReceiptRef,
  modeLeaseAcquisitionReceipt: PreArmCancellationReceiptRef,
  updateProgress: PreArmCancellationUpdateProgress,
  cancellationPersistenceReceipt: PreArmCancellationReceiptRef,
  modeLeaseReleaseReceipt: PreArmCancellationReceiptRef,
  rootGuardReleaseReceipt: PreArmCancellationReceiptRef }`.

This union is the operation's only legal effect order: root acquisition precedes
mode-lease acquisition. Before update readiness, a stopped attempt may
compensate by releasing the mode lease and then the root. On the terminalization
path, an update is applied or proven unnecessary before cancellation
persistence, and cancellation persistence precedes mode-lease then root release.
No release is legal between update readiness and cancellation persistence. It cannot encode a held mode lease
without a held root, an applied update without both acquisition receipts, a
persisted cancellation without update readiness, or root release while the mode
lease remains held. Negative schema fixtures inject every later-stage receipt
into each earlier branch, omit every required prefix receipt, swap release
order, and pair `notRequired` with a non-empty selective plan.
They also reject `alreadyExact` for an empty plan, without its complete guarded
evidence, or with any update-effect receipt.
Every receipt ref inside the original effect progress has
`source=priorOperation`; `source=finalizationPlan` is legal only in the
separately approved finalization receipt plan. A resumed finalization records
its exact realized receipts in separate attempt progress and never overwrites
the original effect observation.

`PreArmCancellationEffectObservation` is the closed record `{
observationId, priorOperationId, supportActionId,
expectedSupportActionDigest, approvedCancellationDigest,
armingReceiptAbsent: true, manualTargetMode: ManualSupportTargetMode,
effectProgress: PreArmCancellationEffectProgress,
historyPartition: RepositoryHistoryPartition,
observedOriginalFingerprint, observedSupportGraphDigest,
observationDigest }`. The mode-lease receipts identify the reserved-original
lease in `reservedOriginal` and the working-IB lease in
`separateWorkingInfobase`; those identities/capabilities are recovered only
from the approved cancellation digest and journal, never caller input. The
history partition is the complete authorization-anchored cancellation range and
contains only the classifications legal for an awaiting action. Progress can
never encode `armed`, and no arming field is inferred. It describes what the
interrupted operation journal proves, not the current task projection: that
projection remains `frozenForRecovery` until approved finalization publishes
the cancellation receipt. This record exists only after every effect through
the selected progress branch and the absence of every later effect are
capability-proven; otherwise the observation-stage recovery plan remains
current with the corresponding typed unknown. Its digest is
`sha256(canonical(observation-without-observationDigest))`.

`PreArmCancellationReceiptPlan` is the closed `{
rootGuardAcquisitionReceipt: PreArmCancellationReceiptRef,
modeLeaseAcquisitionReceipt: PreArmCancellationReceiptRef,
selectiveUpdateDisposition: notRequired | alreadyExact | alreadyApplied |
perform,
selectiveUpdateEffectReceipt?: PreArmCancellationReceiptRef,
cancellationPersistenceReceipt: PreArmCancellationReceiptRef,
modeLeaseReleaseReceipt: PreArmCancellationReceiptRef,
rootGuardReleaseReceipt: PreArmCancellationReceiptRef,
recoveryFinalizationReceipt: PreArmCancellationReceiptRef, receiptPlanDigest }`.
Update/cancellation refs are copied as `priorOperation` whenever those immutable
effects exist. `selectiveUpdateDisposition=notRequired` is exact for an empty
target set; `alreadyExact` is legal only when the immutable effect observation
contains `updateProgress=alreadyExact` under its prior-operation guards;
`alreadyApplied` is exact when that observation contains the immutable applied
update; and `perform` is required for every earlier non-empty stage, even if
the later recheck observes the expected target map. The update ref is absent for the first two dispositions,
`priorOperation` for `alreadyApplied`, and `finalizationPlan` for `perform`.
Thus a non-empty already-exact plan has no update action or invented receipt.
A guard acquisition ref is copied from original progress when it identifies
that exact prior-operation window; its guard is a starting held guard only in a
matching held source stage. A guard release ref is copied exactly when original
progress proves that release already happened; otherwise the release ref is
`finalizationPlan`. Any
compensated attempt that released an inherited or newly acquired guard makes the
next plan allocate new `finalizationPlan` acquisition/release refs from its
effective starting state; neither the original nor the compensated receipt is
reused as a held guard. A guard released during an earlier compensation is not the
finalization guard. The recovery-finalization ref is always `finalizationPlan`; it is the
receipt for publishing the terminal local recovery envelope after every
external cancellation/release postcondition is proven, not a substitute for the
authorization-cancellation receipt. No receipt ID/digest is repeated across different effect kinds, and
`receiptPlanDigest == sha256(canonical(plan-without-receiptPlanDigest))`.

`PreArmCancellationFinalizationRecheckPolicy` is the closed `mode`-tagged
`oneOf` of:

- `replannableBeforeUpdate { mode: replannableBeforeUpdate,
  sourceProgressStage: noGuard | rootHeldBeforeLease |
  rootReleasedBeforeLease | guardsHeldBeforeUpdate |
  modeReleasedBeforeUpdateRootHeld | guardsReleasedBeforeUpdate,
  continuouslyHeldRoot: boolean, continuouslyHeldModeLease: boolean,
  expectedHistoryThroughCursor: RepositoryHistoryCursor,
  expectedHistoryPartitionDigest, expectedOriginalFingerprint,
  expectedSupportGraphDigest, preArmFreezeDigest, policyDigest }`;
- `protectedUpdateReady { mode: protectedUpdateReady,
  sourceProgressStage: updateReadyGuardsHeld,
  expectedHistoryThroughCursor: RepositoryHistoryCursor,
  expectedHistoryPartitionDigest, expectedOriginalFingerprint,
  expectedSupportGraphDigest, preArmFreezeDigest,
  allowedNonRootTailClassifications: [unrelatedRoutine, relevantRoutine],
  alwaysSelectRelevantAdvancePhase: true, policyDigest }`; or
- `releaseOnlyAfterPersistence` is itself the closed
  `sourceProgressStage`-tagged `oneOf` of
  `cancellationPersistedGuardsHeld { mode: releaseOnlyAfterPersistence,
  sourceProgressStage: cancellationPersistedGuardsHeld,
  persistedHistoryThroughCursor: RepositoryHistoryCursor,
  persistedHistoryPartitionDigest, preArmFreezeDigest,
  allowedTailClassifications: [unrelatedRoutine, relevantRoutine],
  alwaysSelectRelevantAdvancePhase: true, policyDigest }`,
  `cancellationPersistedModeReleased { mode: releaseOnlyAfterPersistence,
  sourceProgressStage: cancellationPersistedModeReleased,
  persistedHistoryThroughCursor: RepositoryHistoryCursor,
  persistedHistoryPartitionDigest, preArmFreezeDigest,
  allowedTailClassifications: [unrelatedRoutine, relevantRoutine],
  alwaysSelectRelevantAdvancePhase: true, policyDigest }`, or
  `cancellationPersistedReleased { mode: releaseOnlyAfterPersistence,
  sourceProgressStage: cancellationPersistedReleased,
  persistedHistoryThroughCursor: RepositoryHistoryCursor,
  persistedHistoryPartitionDigest, preArmFreezeDigest,
  allowedTailClassifications: [unrelatedRoutine, relevantRoutine,
  externalSupport, preArmExternal], alwaysSelectRelevantAdvancePhase: true,
  policyDigest }`. The first two branches still have the root guard, so only
  non-root routine history can append. Only the fully released branch can
  classify a subsequent complete root/support version as external support or
  frozen no-arming `preArmExternal` history.

`PreArmCancellationFinalizationReplanMismatchKind` is the closed enum
`nonRootRoutineTailAdvanced | rootOrSupportVersionChanged |
originalTargetChanged | supportGraphChanged`.
`PreArmCancellationFinalizationCapabilityBreachKind` is the closed enum
`historyGap | rootOrSupportVersionChanged | originalTargetChanged |
supportGraphChanged | rootGuardLost | modeLeaseLost | receiptOwnerMismatch`.
`PreArmCancellationFinalizationRecheckEvidence` is the closed `outcome`-tagged
`oneOf` of `matched { outcome: matched, observedHistoryPartition,
observedOriginalFingerprint, observedSupportGraphDigest, evidenceDigest }`,
`safeTailExtended { outcome: safeTailExtended, baseHistoryPartition,
appendedNonRootHistoryPartition, combinedHistoryPartition,
observedOriginalFingerprint,
observedSupportGraphDigest, relevantAdvanceSelected: true, evidenceDigest }`,
`releaseTailObserved { outcome: releaseTailObserved,
persistedHistoryPartition, appendedHistoryPartition,
observedOriginalFingerprint, observedSupportGraphDigest,
relevantAdvanceSelected: true, evidenceDigest }`,
`replanRequired { outcome: replanRequired, mismatchKinds:
PreArmCancellationFinalizationReplanMismatchKind[], refreshedHistoryPartition,
observedOriginalFingerprint, observedSupportGraphDigest, evidenceDigest }`, or
`capabilityBreach { outcome: capabilityBreach, mismatchKinds:
PreArmCancellationFinalizationCapabilityBreachKind[], observedHistoryPartition?,
observedOriginalFingerprint?, observedSupportGraphDigest?, evidenceDigest }`.
Every list is non-empty/canonical where named and each evidence digest hashes
the closed branch without itself. Every full observed/refreshed/persisted
partition begins at the authorization cursor. For `safeTailExtended`,
`baseHistoryPartition` is byte-identical to
`finalizationPlan.finalizationHistoryPartition`, the non-empty
`appendedNonRootHistoryPartition` begins at its endpoint, and
`combinedHistoryPartition` is the exact concatenation of the base entries and
appended entries with the base lower bound and appended endpoint. Its digest is
therefore independently recomputed over that exact concatenation; no observed
entry is replaced or lost. `appendedHistoryPartition` begins at
`persistedHistoryPartition`'s endpoint and may be the canonical empty partition
exactly when its two endpoints are equal. Otherwise it is non-empty. Both
appended partitions are contiguous and contain exactly the classifications
allowed by their stage-specific policy; neither can hide or duplicate an
entry.

The enclosing plan, policy, and full evidence partitions have one exact
binding. For `replannableBeforeUpdate` and `protectedUpdateReady`,
`expectedHistoryThroughCursor == finalizationHistoryPartition.throughInclusive`
and `expectedHistoryPartitionDigest ==
finalizationHistoryPartition.partitionDigest`. For every
`releaseOnlyAfterPersistence` branch, `persistedHistoryThroughCursor ==
finalizationHistoryPartition.throughInclusive` and
`persistedHistoryPartitionDigest ==
finalizationHistoryPartition.partitionDigest`. A `matched` outcome requires
`observedHistoryPartition` byte-for-byte equal to
`finalizationPlan.finalizationHistoryPartition`; a `releaseTailObserved`
outcome requires `persistedHistoryPartition` byte-for-byte equal to that same
plan partition. Cursor/digest equality without full record equality is
insufficient, so neither evidence nor a terminal result can splice an
independently supplied partition behind a matching hash field.

For `replannableBeforeUpdate`, a fresh read under both guards must match the
frozen authorization and all expected fields before any update/cancellation.
A conclusive mismatch performs neither effect, releases the mode lease then
root with the attempt's exact receipts, freezes the complete compensated attempt
as audit lineage, and publishes a fresh plan/digest with `replanRequired` for
explicit reapproval. For the three mutable-state kinds this is legal only when
the mismatched state could have changed while its corresponding guard was
absent. A `replanRequired` branch is legal only for those three conclusive kinds or the exact
`nonRootRoutineTailAdvanced` case and must carry the complete
authorization-anchored `refreshedHistoryPartition`; a history-coverage gap is
never a refreshed partition. `historyGap`, either lost guard, and receipt-owner
mismatch are always `capabilityBreach`. The three mutable-state mismatch kinds
also become `capabilityBreach`, never replan, when their corresponding guard was
continuous. Root/support-version and support-graph state correspond to the root
guard; selected-original-target state corresponds to the mode lease.

`nonRootRoutineTailAdvanced` is present exactly when the refreshed partition is
the finalization plan's partition followed by a non-empty, contiguous append:
both lower bounds are equal, the plan entries are an exact unmodified prefix of
the refreshed entries, the suffix begins at the plan endpoint, and every suffix
entry is `unrelatedRoutine` or `relevantRoutine`. All history coverage is
complete. It is eligible independently of both continuity
literals, including after a real release/reacquisition gap, because neither
guard serializes non-root repository commits. It is not
`safeTailExtended`: before update the attempt performs no update or
cancellation, compensates mode then root, appends the receipt-proven attempt
audit, and publishes the refreshed-partition plan under a new digest for
explicit reapproval. The next plan preserves its prior `plannedResultPhase`
for an all-unrelated append and uses `relevantAdvancePhase` when the exact
appended suffix contains any `relevantRoutine` entry.

The two continuity literals are derived from the actual finalization plan, not
from the immutable source progress stage:
`continuouslyHeldRoot == (startingRootGuardState == heldFromPriorOperation &&
acquireRootGuard == false)` and
`continuouslyHeldModeLease == (startingModeLeaseState ==
heldFromPriorOperation && acquireModeLease == false)`. A newly acquired guard
protects the current recheck but does not erase the release gap before that
acquisition. Consequently every fresh plan after a compensated attempt has
both literals false even when its immutable `sourceProgressStage` originally
named held guards. A root/support version observed after a real root-release
gap is classified as `preArmExternal` with the same frozen no-arming identity.
For
`protectedUpdateReady`, the root/mode guards and any immutable update receipt
already form a protected boundary: only a complete non-root routine tail may be
accepted as `safeTailExtended`, and the plan pessimistically keeps
`relevantAdvancePhase`. Root/support, selected-target, guard, lease, or owner
drift under that boundary is `capabilityBreach` and remains unknown-effect
recovery; it can never be rewritten as a new no-effect plan. The third branch
is required after cancellation persistence: cancellation/update cannot be
undone or repeated, so only the missing release suffix and local terminal
envelope remain. Its successful recheck uses `releaseTailObserved`; that
branch keeps the complete allowed appended partition separate from the
persisted cancellation partition. It records the complete append-only tail when available and
always chooses the bound relevant-advance phase; a coverage gap is retained as
`DeferredRepositoryAdvance/coverageUnknown`. Each `policyDigest` covers every
field except itself and excludes the enclosing finalization digest.

`PreArmCancellationFinalizationExecutionPath` is the closed `pathKind`-tagged
`oneOf` of `success { pathKind: success, actionIds: UnicaId[] }`,
`capabilityBreachStop { pathKind: capabilityBreachStop, actionIds:
UnicaId[] }`, `rootGuardConflictCompensation { pathKind:
rootGuardConflictCompensation, actionIds: UnicaId[] }`,
`modeLeaseUnavailableBeforeAcquisitionCompensation { pathKind:
modeLeaseUnavailableBeforeAcquisitionCompensation, actionIds: UnicaId[] }`,
`modeLeaseUnavailableAfterAcquisitionCompensation { pathKind:
modeLeaseUnavailableAfterAcquisitionCompensation, actionIds: UnicaId[] }`, or
`recheckReplanCompensation { pathKind: recheckReplanCompensation, actionIds:
UnicaId[] }`. `PreArmCancellationFinalizationExecutionPathPlan` is the closed
`{ paths: PreArmCancellationFinalizationExecutionPath[],
executionPathPlanDigest }`. Paths are canonical/unique by kind, every action
list is non-empty and duplicate-free, and each ID resolves to exactly one
action in the enclosing `RecoveryPlanStatus.actions` catalog. There is exactly
one `success` path and one `capabilityBreachStop` path. The applicable known-
failure compensation paths are present exactly as follows: root conflict iff a
root acquisition is planned; mode-unavailable-before-acquisition iff a mode
acquisition is planned; mode-unavailable-after-acquisition iff the selected
mode can acquire its lease before returning a conclusive dirty/busy stop; and
recheck-replan for every `replannableBeforeUpdate` policy, because a complete
non-root routine tail can require reapproval even when both continuity literals
are true. The path-plan digest hashes the closed record without itself.

The success path is the receipt-derived acquisition, recheck, update,
cancellation, release, and finalization order specified below. The capability-
breach path is its acquisition prefix followed by the recheck and then stops.
The root-conflict compensation path contains only the attempted root acquire.
The mode-unavailable-before-acquisition path contains the optional successful
root acquire, the attempted mode acquire, and root release. The corresponding
after-acquisition path contains every successful planned acquisition action,
then mode release and root release. The recheck-replan path contains every
planned acquisition, the recheck, then mode release and root release. Inherited
acquisitions generate no action in any path, while their release actions remain
present when that path must release them. No compensation path contains the
selective update, cancellation persistence, or local recovery-finalization
action. A conclusive runtime selects exactly one listed path; an unknown action
effect instead retains the current progress and typed unknown without
pretending that a path completed. Thus reverse releases are approved branches,
not a linear success list with update/cancellation actions silently skipped.

`PreArmCancellationModeLeaseCompensation` is the closed
`modeLeaseAcquisitionState`-tagged `oneOf` of `notAcquired {
modeLeaseAcquisitionState: notAcquired, selectedExecutionPathKind:
modeLeaseUnavailableBeforeAcquisitionCompensation,
realizedForwardReceipts: PreArmCancellationEffectReceipt[],
compensationReleaseReceipts: PreArmCancellationEffectReceipt[] }` or
`acquired { modeLeaseAcquisitionState: acquired,
selectedExecutionPathKind:
modeLeaseUnavailableAfterAcquisitionCompensation,
realizedForwardReceipts: PreArmCancellationEffectReceipt[],
compensationReleaseReceipts: PreArmCancellationEffectReceipt[] }`.
`PreArmCancellationFinalizationCompensatingCause` is the closed
`stopCause`-tagged `oneOf` of `modeLeaseUnavailable { stopCause:
modeLeaseUnavailable, modeLease: PreArmCancellationModeLeaseCompensation }` or
`recheckReplanRequired { stopCause: recheckReplanRequired,
selectedExecutionPathKind: recheckReplanCompensation,
realizedForwardReceipts: PreArmCancellationEffectReceipt[],
compensationReleaseReceipts: PreArmCancellationEffectReceipt[],
recheckEvidence: PreArmCancellationFinalizationRecheckEvidence }`.
`PreArmCancellationFinalizationCompensatedCause` is the closed
`stopCause`-tagged `oneOf` of `rootGuardConflict { stopCause:
rootGuardConflict, selectedExecutionPathKind: rootGuardConflictCompensation,
realizedForwardReceipts: [], compensationReleaseReceipts: [] }`,
`modeLeaseUnavailable { stopCause: modeLeaseUnavailable,
modeLease: PreArmCancellationModeLeaseCompensation }`, or
`recheckReplanRequired { stopCause: recheckReplanRequired,
selectedExecutionPathKind: recheckReplanCompensation,
realizedForwardReceipts: PreArmCancellationEffectReceipt[],
compensationReleaseReceipts: PreArmCancellationEffectReceipt[],
recheckEvidence: PreArmCancellationFinalizationRecheckEvidence }`.

`PreArmCancellationFinalizationAttemptProgress` is the closed `attemptState`-
tagged `oneOf` of `notStarted { attemptState: notStarted,
finalizationAttemptId }`, `inProgress { attemptState: inProgress,
finalizationAttemptId, selectedExecutionPathKind: success |
capabilityBreachStop, realizedForwardReceipts:
PreArmCancellationEffectReceipt[], recheckEvidence?:
PreArmCancellationFinalizationRecheckEvidence }`, `compensating {
attemptState: compensating, finalizationAttemptId, compensation:
PreArmCancellationFinalizationCompensatingCause }`, `compensated {
attemptState: compensated, finalizationAttemptId, compensation:
PreArmCancellationFinalizationCompensatedCause,
allAttemptGuardsReleased: true, attemptAuditDigest }`, or `completed {
attemptState: completed, finalizationAttemptId,
selectedExecutionPathKind: success, realizedReceipts:
PreArmCancellationEffectReceipt[], recheckEvidence:
PreArmCancellationFinalizationRecheckEvidence, allAttemptGuardsReleased: true,
attemptAuditDigest }`.

For `inProgress`, `realizedForwardReceipts` is a proper canonical effect-order
prefix of this attempt's `source=finalizationPlan` success refs, materialized
one-to-one as full effect receipts; inherited refs are never duplicated. Before
the recheck it contains only the planned acquisition-receipt prefix and
`recheckEvidence` is absent. After a successful recheck its evidence is present
and is exactly `matched` for `replannableBeforeUpdate`, `matched` or
`safeTailExtended` for `protectedUpdateReady`, and `releaseTailObserved` for
`releaseOnlyAfterPersistence`; only then may the forward prefix extend into
update, cancellation, or success releases. `capabilityBreachStop` requires
`outcome=capabilityBreach`, the exact complete planned-acquisition receipt
prefix, and no update/cancellation/release/finalization receipt. A
`replanRequired` outcome is never represented as `inProgress`.

The compensation branch name, `stopCause`, evidence presence, and receipt lists
are an exact discriminator mapping. `rootGuardConflictCompensation` is legal
only as already `compensated`: `stopCause=rootGuardConflict`, both receipt lists
are empty, and recheck evidence and mode-acquisition state are absent.
`modeLeaseUnavailableBeforeAcquisitionCompensation` has
`stopCause=modeLeaseUnavailable`, `modeLeaseAcquisitionState=notAcquired`, no
recheck evidence, and a forward list containing exactly the newly acquired root
receipt iff root acquisition was planned; its required compensation list is
exactly `[rootGuardRelease]`. The after-acquisition variant instead has
`modeLeaseAcquisitionState=acquired`, the exact complete planned-
acquisition receipt list, no recheck evidence, and required compensation list
`[modeLeaseRelease, rootGuardRelease]`; its stop evidence's lease receipt IDs
equal those acquire/release effect receipts. `recheckReplanCompensation` has
`stopCause=recheckReplanRequired`, no mode-acquisition-state field, the exact
complete planned-acquisition receipt list, required `outcome=replanRequired`
evidence, and required compensation list
`[modeLeaseRelease, rootGuardRelease]`. `compensating` carries a proper prefix
of the applicable required compensation list; `compensated` carries that list
in full and proves `allAttemptGuardsReleased`. Neither branch can contain an
update, cancellation, or finalization receipt, and a compensation receipt is
never placed in the forward list.

`completed.realizedReceipts` contains every planned success receipt not already
inherited, ends with `recoveryFinalization`, and has exactly the mode-specific
successful recheck outcome listed above. Each audit digest hashes its closed
branch without itself.

`PreArmCancellationFinalizationAttemptAudit` is the closed `{
finalizationAttemptId, finalizationPlanDigest, compensatedProgress:
PreArmCancellationFinalizationAttemptProgress, auditDigest }`; its progress is
exactly the `compensated` branch and `auditDigest` hashes the record without
itself. `compensatedProgress.finalizationAttemptId == finalizationAttemptId`;
`finalizationPlanDigest` is the exact immutable plan whose receipt refs/actions
produced that progress, and every receipt intent carries the same attempt ID.
It preserves new attempt receipts without overwriting or dropping the
original `PreArmCancellationEffectObservation` compensation chain.

`PreArmCancellationKnownBlocker` is the closed `blockerKind`-tagged `oneOf`
of `rootGuardConflict { blockerKind: rootGuardConflict,
previousRecoveryDigest, compensatedAttemptAuditDigest,
failedTarget: RepositoryTargetIdentity, failedTargetDisplay: RepositoryTargetDisplay,
lockedBy: RepositoryOwnerIdentity | null,
requiredExternalAction: ReleaseRepositoryLocksInstruction, blockerDigest }`
or `modeLeaseUnavailable { blockerKind: modeLeaseUnavailable,
previousRecoveryDigest, compensatedAttemptAuditDigest,
manualTargetMode: ManualSupportTargetMode,
workingInfobaseStop?: ManualWorkingInfobaseStopEvidence,
reservedOriginalLeaseStop?: ReservedOriginalLeaseStopEvidence,
requiredExternalAction: CleanManualWorkingInfobaseInstruction |
CloseReservedOriginalDesignerInstruction, blockerDigest }`. The mode branch
uses the ordinary exclusive mode/stop/instruction presence rule. In either
branch `compensatedAttemptAuditDigest` equals the last audit in the fresh
finalization plan's append-only `priorAttemptAudits`; that audit's stop cause,
receipts, failed mode, and instruction evidence match the blocker branch. The
old approved recovery digest is retained only as `previousRecoveryDigest`.
`blockerDigest == sha256(canonical(blocker-without-blockerDigest))`; the blocker
contains neither the fresh `RecoveryPlanStatus` nor its recovery/finalization
digest, so persisting it in that plan creates no hash cycle.

The effect observation, progress, audit, blocker, and finalization-plan records
are closed values, not standalone execution authority. A producer may
materialize them only while validating the current immutable plan, exact
selected path, receipt refs, attempt lineage, mode-specific stop evidence, and
required instruction as one operation. A schema-valid record or a constructor
that receives only the record itself cannot authorize an effect or publish
terminal progress.

`PreArmCancellationFinalizationPlan` is the closed `{
finalizationAttemptId, priorOperationId, supportActionId, expectedSupportActionDigest,
approvedCancellationDigest, effectObservationDigest,
completionMode: verifyCancelledAndRelease | finishCancellationAndRelease,
manualTargetMode: ManualSupportTargetMode,
startingRootGuardState: heldFromPriorOperation | released,
startingModeLeaseState: heldFromPriorOperation | released,
acquireRootGuard: boolean, acquireModeLease: boolean,
receiptPlan: PreArmCancellationReceiptPlan,
recheckPolicy: PreArmCancellationFinalizationRecheckPolicy,
executionPathPlan: PreArmCancellationFinalizationExecutionPathPlan,
priorAttemptAudits: PreArmCancellationFinalizationAttemptAudit[],
finalizationHistoryPartition: RepositoryHistoryPartition,
selectiveUpdatePlan: SelectiveRepositoryUpdatePlan,
expectedFinalOriginalFingerprint, expectedFinalSupportGraphDigest,
plannedResultPhase, relevantAdvancePhase, finalizationPlanDigest }`. `verifyCancelledAndRelease` is
legal exactly for one of the three `cancellationPersisted*` progress branches;
both acquire flags are then false, `priorAttemptAudits` is exactly `[]`, the
recheck mode is `releaseOnlyAfterPersistence`, and terminal phase is
unconditionally `relevantAdvancePhase`. Its source-stage projection is exact:

- `cancellationPersistedGuardsHeld` starts with both guards
  `heldFromPriorOperation`; both acquisition refs, any `applied` update ref,
  and the cancellation ref are `priorOperation`, both release refs and the
  recovery-finalization ref are `finalizationPlan`, and the success actions are
  exactly recheck, mode release, root release, finalization;
- `cancellationPersistedModeReleased` starts with the root
  `heldFromPriorOperation` and the mode lease `released`; both acquisitions,
  any `applied` update, cancellation, and mode release are `priorOperation`,
  root release and recovery finalization are `finalizationPlan`, and the
  success actions are exactly recheck, root release, finalization; and
- `cancellationPersistedReleased` starts with both states `released`; both
  acquisitions, any `applied` update, cancellation, and both releases are
  `priorOperation`, only recovery finalization is `finalizationPlan`, and the
  success actions are exactly recheck, finalization.

In all three projections the update ref is absent instead of `priorOperation`
for `updateProgress=notRequired` or `alreadyExact`; the receipt-plan
disposition is respectively `notRequired`, `alreadyExact`, or
`alreadyApplied`. No persisted-stage plan can allocate an update/cancellation
receipt or acquisition action. These exact projections are why a release-only
plan cannot have a compensated prior attempt audit.

`finishCancellationAndRelease` is legal exactly for any earlier branch. With no
prior attempt audit, its starting-state/flag projection
is: `noGuard`, `rootReleasedBeforeLease`, and
`guardsReleasedBeforeUpdate` start released and acquire both;
`rootHeldBeforeLease` and `modeReleasedBeforeUpdateRootHeld` start with only the
prior root and acquire only the mode lease; `guardsHeldBeforeUpdate` and
`updateReadyGuardsHeld` start with both prior guards and acquire neither. Every
compensated prior attempt has receipt-proven release of both its newly acquired
and inherited held guards, so the next plan starts both `released`, allocates
new root/mode acquisition and release refs, and sets both acquire flags true
regardless of the immutable original observation. A held mode state without a
held root is invalid. `priorAttemptAudits` is unique by attempt ID, ordered by
durable attempt creation, and is exactly the append-only compensated-attempt
prefix from the preceding plan; a fresh plan may append only the immediately
preceding compensated attempt and may never remove/reorder/substitute one. The
first plan's `finalizationHistoryPartition` equals the effect observation's
partition byte-for-byte. After `replanRequired`, the next plan's
`finalizationHistoryPartition` is instead byte-for-byte equal to that evidence's
complete authorization-anchored `refreshedHistoryPartition`; its lower bound is
unchanged and its endpoint/digest equal the new recheck policy fields. For
`nonRootRoutineTailAdvanced`, its planned result phase follows the exact
all-unrelated/relevant-suffix rule above. Its
recheck mode is `protectedUpdateReady` exactly for `updateReadyGuardsHeld` and
`replannableBeforeUpdate` otherwise. The exact approved update is absent only
when the immutable observation already proves `applied`, `alreadyExact`, or an
empty target set; every earlier non-empty stage uses disposition `perform`.
Existing held guards are reused solely for the first un-compensated plan.
Whenever finalization itself performs cancellation, the mode-specific lease
remains held through durable authorization cancellation and is released before
the root guard. A release-only plan never reacquires a guard already released
by the prior operation. The plan can
update only the original configuration's exact selective root target; it never
creates a repository version, changes support settings, or grants edit
authority. `plannedResultPhase` is the approved cancellation preview's bound
`cancelledPhase` or `relevantAdvancePhase`, with `relevantRoutine`,
`externalSupport`, or `preArmExternal` history forcing the latter.
The phase pair comes only from the digest-validated approved-cancellation
authority bound by `approvedCancellationDigest`; the effect-observation wire
record and its digest do not duplicate either phase field.
`protectedUpdateReady` also unconditionally requires
`plannedResultPhase == relevantAdvancePhase`, matching its pessimistic policy;
no schema-valid byte-identical terminal can select `cancelledPhase`. The plan digest covers every field except itself.
The execution-path plan contains only action IDs, not action digests or the
enclosing finalization digest. Each action independently repeats the complete
`finalizationPlanDigest`, so allocation and hashing have no cycle.
An observation that disagrees with the approved cancellation partition or
cannot match exactly one progress branch is a capability/state-corruption
recovery finding and cannot materialize either finalization mode.
`finalizationAttemptId` is allocated by the server when the finalize plan is
materialized and is covered by its digest; the later caller-stable recovery
`operationId` binds to this attempt but is not needed in any planned
receipt hash.

When present, `recovery` is the complete closed `RecoveryPlanStatus {
priorOperationId, target, effectClass, plannedResultPhase,
observations: RecoveryObservation[], actions: RecoveryAction[],
supportVersionObservations?: SupportPrerequisiteVersionObservation[],
supportHistoryFromCursor?: RepositoryHistoryCursor,
supportHistoryThroughCursor?: RepositoryHistoryCursor,
supportHistoryPartition?: RepositoryHistoryPartition,
supportRecoveryDisposition?: SupportRecoveryDisposition,
supportLateRelevantResultPhase?: TaskPhase,
successfulIntegrationForbidden?: true,
supportRecoveryFinalizationPlan?: SupportRecoveryFinalizationPlan,
latestSupportRecoveryGuardProof?: SupportRecoveryGuardProof,
manualWorkingInfobaseClosurePlan?: ManualWorkingInfobaseClosurePlan,
requiredExternalAction?: SupportRecoveryExternalAction,
preArmCancellationStage?: observeOutcome | finalize,
preArmCancellationEffectObservation?: PreArmCancellationEffectObservation,
preArmCancellationFinalizationPlan?: PreArmCancellationFinalizationPlan,
preArmCancellationFinalizationProgress?:
PreArmCancellationFinalizationAttemptProgress,
preArmCancellationKnownBlocker?: PreArmCancellationKnownBlocker,
remainingUnknowns: RecoveryUnknown[],
recoveryDigest }`, not only the compact resume handle.
`target` is `taskConfiguration`, `repositoryLocks`, `originalConfiguration`,
`repositoryCommit`, `supportPrerequisite`, `preArmSupportCancellation`,
`manualWorkingInfobaseLease`,
`artifact`, `archive`, or `cleanup`;
`manualWorkingInfobaseLease` is legal only for pre-authorization baseline work:
the plan has no support action fields and no support action has been published.
Every unknown lease effect after publication uses `target:
supportPrerequisite` for an armed action or `target:
preArmSupportCancellation` for an interrupted awaiting-action cancellation. It
preserves the exact action lineage and follows the matching closed presence
rules below.
`effectClass` is
`compensate`, `rollback`, `reconcileOnly`, `quarantine`, or `cleanup`.
`RecoveryPlanStatus` is a closed target/effect tagged union whose action array
is further discriminated by the following exhaustive mapping; no cross-row
action kind, permutation, or duplicate is schema-valid:

| Target / effect class | Required ordered action grammar |
| --- | --- |
| `taskConfiguration / rollback` | exactly one of `restoreTaskCheckpoint`, `recreateTaskInfobase`, or observation-only `verifyTaskFingerprint`; restore/recreate already require the terminal fingerprint observation and are not followed by a duplicate verifier |
| `repositoryLocks / compensate` | exactly `releaseOwnedLocks` |
| `originalConfiguration / rollback` | `restoreOriginal`, optionally followed by `releaseOwnedLocks` when the recorded operation owns locks |
| `repositoryCommit / reconcileOnly` | tagged `observeOutcome` contains exactly `observeCommit` and can only publish a freshly digest-bound branch plan; tagged `committed` binds that prior positive observation and contains exactly `releaseOwnedLocks` when locks remain |
| `repositoryCommit / rollback` | tagged `notCommitted` binds the prior negative/known-failed observation and contains `restoreOriginal` followed by `releaseOwnedLocks`; it proves the checkpoint fingerprint before the safe phase |
| `supportPrerequisite / reconcileOnly` | one or more support-history/required external-evidence observations, then mode-specific lease observe/release and optional `updateOriginalSelectedTargets`, then exactly `finalizeSupportPrerequisiteRecovery`; waits stop before terminal finalization |
| `preArmSupportCancellation / reconcileOnly` | tagged `observeOutcome` contains exactly `observePreArmCancellationOutcome` and can only publish a freshly digest-bound `finalize` plan. Tagged `finalize` uses the closed `PreArmCancellationFinalizationExecutionPathPlan`: `actions[]` is the canonical de-duplicated action catalog in success-path order, while each selected success, capability-breach stop, or compensation path is one exact listed action-ID sequence. The success sequence contains `acquirePreArmRootGuard` iff its acquisition ref is `finalizationPlan`; `acquirePreArmModeLease` iff its ref is `finalizationPlan`; exactly `recheckPreArmCancellationFinalization`; `applyPreArmCancellationSelectiveUpdate` iff `selectiveUpdateDisposition=perform`; `persistPreArmSupportCancellation` iff the cancellation ref is `finalizationPlan`; `releasePreArmModeLease` iff its release ref is `finalizationPlan`; `releasePreArmRootGuard` iff its release ref is `finalizationPlan`; and exactly `finishPreArmCancellationRecovery` last. Compensation sequences are the exact reverse-release branches defined above and omit update/cancellation/finalization by construction, rather than skipping entries of a linear success execution. Every effect action maps to exactly one receipt-plan ref/outcome; prior-operation receipts generate no duplicate action |
| `manualWorkingInfobaseLease / reconcileOnly` | `observeWorkingInfobaseLease`, followed by `releaseWorkingInfobaseLease` iff the pre-authorization lease is held |
| `artifact / quarantine` | `quarantineArtifact` or `resumeQuarantine`, followed by exact presence observation through its action projection |
| `archive / cleanup` | exactly `observeArchiveStaging`, then one `observeRetentionLease`, `releaseRetentionLease` pair for every lease in canonical handoff order, then exactly `finishArchive` |
| `cleanup / cleanup` | one or more `resumeOwnedTargetQuarantine` actions in canonical owned-target order, then exactly `finishCleanup` |

An empty owned-target set requires no cleanup-recovery plan and completes
directly; it cannot manufacture the otherwise mandatory non-empty action
postconditions for `finishCleanup`. Every other target/effect pair is rejected
by the schema snapshot. The armed
support-prerequisite variant additionally carries its disposition and manual
target mode, so only working-IB actions are legal in separate mode and only
reserved-original lease actions in reserved mode. Negative schema tests inject
every action kind into every other row and permute required terminal actions.
The pre-arm variant instead binds the interrupted cancellation operation and
has no disposition, corrective instruction, support-recovery distribution, or
arming receipt. Its `observeOutcome` stage never publishes a lifecycle success.
A conclusive effect observation replaces it with the digest-distinct `finalize`
plan and returns `recoveryReapprovalRequired`; no external effect begins until
that new plan is explicitly approved. An unknown observation keeps the old plan
current. The finalizer either verifies an already durable cancellation and
releases its exact retained guards or completes the already-approved selective
original update and cancellation under those guards. It cannot arm, consume, or
repair support transitions.
The commit `observeOutcome` branch never publishes a lifecycle success. A
conclusive observation durably creates either the `committed` release-only plan
or the `notCommitted` original-restore/full-release plan and stops with generic
`recoveryReapprovalRequired`; the new digest must be approved separately. A
still-unknown observation keeps the old plan current. `repositoryCommitFailed`
may start directly with the `notCommitted` branch only when capability evidence
already proves zero task commit. No approved action array changes branches at
runtime. The committed branch plans `committedAndUnlocked`; the not-committed
branch invalidates main-integration descendants and plans `synchronized` only
after checkpoint fingerprint restoration and complete release are proven.
`RecoveryObservationKind` is the closed enum `repositoryAnchor`,
`repositoryVersion`, `supportGraph`, `supportActionAuthorization`,
`objectFingerprint`, `taskFingerprint`, `lockOwnership`,
`workingInfobaseLease`, `reservedOriginalLease`, `retentionLease`,
`finalizationPolicy`,
`artifactPresence`, `archiveStagingPresence`, `archivePresence`, or
`quarantinePresence`. `RecoveryObservation` is the closed tagged `oneOf` of:

- `matched { outcome: matches, observationKind: RecoveryObservationKind,
  subject: RecoverySubjectRef, expectedDigest, observedDigest,
  observationDigest }`, where both
  digests are equal;
- `differed { outcome: differs, observationKind: RecoveryObservationKind,
  subject: RecoverySubjectRef, expectedDigest, observedDigest,
  observationDigest }`, where both are present and
  unequal; or
- `unknown { outcome: unknown, observationKind: RecoveryObservationKind,
  subject: RecoverySubjectRef, expectedDigest, observedDigest: null, unknownReason:
  observationUnavailable | capabilityUnproven | effectOutcomeUnavailable,
  observationDigest }`.

No outcome permits an omitted digest or a caller-chosen match result.
For every variant, `observationDigest ==
sha256(canonical(observation-without-observationDigest))`.
`RecoverySubjectRef` is the closed tagged `oneOf` of `registered { subjectKind:
registered, subjectId: UnicaId }`, `metadataObject { subjectKind:
metadataObject, objectId: MetadataObjectId }`, `configurationRoot {
subjectKind: configurationRoot }`, or `ownedRole { subjectKind: ownedRole,
locator: OwnedTargetLocator }`, `externalWorkingInfobase { subjectKind:
externalWorkingInfobase, identity: ManualWorkingInfobaseIdentity }`,
`reservedOriginalInfobase { subjectKind: reservedOriginalInfobase,
originalIdentityDigest: Sha256 }`, or `retentionLease { subjectKind:
retentionLease, retentionLeaseId: UnicaId }`.
`RecoveryExpectedObservation` is the closed `{ observationKind:
RecoveryObservationKind, subject: RecoverySubjectRef, expectedDigest }`.
`ArchiveStagingReceipt` is the closed `{ stagingReceiptId, archiveId,
handoffLineageDigest, frozenProviderBoundaryDigest, stagedArchiveSha256,
fileSynced: true, parentDirectorySynced: true, durableWriteReceiptId,
receiptDigest }`; `receiptDigest ==
sha256(canonical(receipt-without-receiptDigest))`. It proves the complete
handoff/evidence lineage is durably recoverable before any external retention
lease is released; it is not final archive publication.
`RecoveryAction` is the following closed tagged `oneOf` (every variant also
requires `actionId: UnicaId`, `expectedObservations:
RecoveryExpectedObservation[]`, `expectedPostconditionDigest: Sha256`, and
`actionDigest: Sha256`):

- `releaseOwnedLocks { actionKind: releaseOwnedLocks, subjects:
  RecoverySubjectRef[], expectedOwnedLockSetDigest }`;
- `restoreOriginal { actionKind: restoreOriginal, checkpointId,
  expectedOriginalFingerprint }`;
- `restoreTaskCheckpoint { actionKind: restoreTaskCheckpoint, checkpointId,
  expectedTaskFingerprint }`;
- `recreateTaskInfobase { actionKind: recreateTaskInfobase,
  sourceCheckpointId, expectedTaskFingerprint }`;
- `verifyTaskFingerprint { actionKind: verifyTaskFingerprint,
  expectedTaskFingerprint }`;
- `observeCommit { actionKind: observeCommit, operationId, integrationSetId,
  expectedIntegrationSetDigest }`;
- `observePreArmCancellationOutcome { actionKind:
  observePreArmCancellationOutcome, priorOperationId, supportActionId,
  expectedSupportActionDigest, approvedCancellationDigest }`;
- `acquirePreArmRootGuard { actionKind: acquirePreArmRootGuard,
  finalizationAttemptId, finalizationPlanDigest, supportActionId,
  receiptRef: PreArmCancellationReceiptRef }`;
- `acquirePreArmModeLease { actionKind: acquirePreArmModeLease,
  finalizationAttemptId, finalizationPlanDigest, supportActionId,
  manualTargetMode: ManualSupportTargetMode,
  reservedOriginalIdentityDigest?, workingInfobaseIdentity?:
  ManualWorkingInfobaseIdentity, exclusiveLeaseCapabilityId,
  receiptRef: PreArmCancellationReceiptRef }`;
- `recheckPreArmCancellationFinalization { actionKind:
  recheckPreArmCancellationFinalization, finalizationAttemptId,
  finalizationPlanDigest, effectObservationDigest, recheckPolicyDigest }`;
- `applyPreArmCancellationSelectiveUpdate { actionKind:
  applyPreArmCancellationSelectiveUpdate, finalizationAttemptId,
  finalizationPlanDigest, selectiveUpdatePlanDigest,
  expectedTargetRevisionMapDigest, receiptRef:
  PreArmCancellationReceiptRef }`;
- `persistPreArmSupportCancellation { actionKind:
  persistPreArmSupportCancellation, finalizationAttemptId, supportActionId,
  expectedSupportActionDigest, approvedCancellationDigest,
  effectObservationDigest, finalizationPlanDigest, receiptRef:
  PreArmCancellationReceiptRef }`;
- `releasePreArmModeLease { actionKind: releasePreArmModeLease,
  finalizationAttemptId, finalizationPlanDigest, manualTargetMode:
  ManualSupportTargetMode, reservedOriginalIdentityDigest?,
  workingInfobaseIdentity?: ManualWorkingInfobaseIdentity,
  exclusiveLeaseCapabilityId, receiptRef: PreArmCancellationReceiptRef }`;
- `releasePreArmRootGuard { actionKind: releasePreArmRootGuard,
  finalizationAttemptId, finalizationPlanDigest, supportActionId,
  receiptRef: PreArmCancellationReceiptRef }`;
- `finishPreArmCancellationRecovery { actionKind:
  finishPreArmCancellationRecovery, finalizationAttemptId, supportActionId,
  expectedSupportActionDigest, approvedCancellationDigest,
  effectObservationDigest, finalizationPlanDigest, receiptPlanDigest,
  expectedResultPhase, receiptRef: PreArmCancellationReceiptRef }`;
- `quarantineArtifact { actionKind: quarantineArtifact, artifactId,
  expectedArtifactSha256, quarantineId }`;
- `observeSupportPrerequisiteHistory { actionKind:
  observeSupportPrerequisiteHistory, supportActionId, fromCursor:
  RepositoryHistoryCursor, throughCursor: RepositoryHistoryCursor,
  expectedPartitionDigest }`;
- `updateOriginalSelectedTargets { actionKind:
  updateOriginalSelectedTargets, finalizationPlanDigest,
  selectiveUpdatePlanDigest, expectedTargetRevisionMapDigest }`;
- `observeWorkingInfobaseLease { actionKind: observeWorkingInfobaseLease,
  workingInfobaseIdentity: ManualWorkingInfobaseIdentity,
  exclusiveLeaseCapabilityId, expectedLeaseState: available | exclusivelyHeld
  | released }`;
- `releaseWorkingInfobaseLease { actionKind: releaseWorkingInfobaseLease,
  workingInfobaseIdentity: ManualWorkingInfobaseIdentity,
  exclusiveLeaseCapabilityId, exclusiveLeaseReceiptId,
  expectedReleaseReceiptId }`;
- `observeReservedOriginalLease { actionKind: observeReservedOriginalLease,
  reservedOriginalIdentityDigest, exclusiveLeaseCapabilityId,
  expectedLeaseState: available | exclusivelyHeld | released }`;
- `releaseReservedOriginalLease { actionKind: releaseReservedOriginalLease,
  reservedOriginalIdentityDigest, exclusiveLeaseCapabilityId,
  exclusiveLeaseReceiptId, expectedReleaseReceiptId }`;
- `observeRetentionLease { actionKind: observeRetentionLease,
  retentionLeaseId, retentionCapabilityRowId,
  expectedLeaseState: held | released }`;
- `observeArchiveStaging { actionKind: observeArchiveStaging,
  archiveStagingReceiptId, expectedArchiveStagingReceiptDigest,
  handoffLineageDigest }`;
- `releaseRetentionLease { actionKind: releaseRetentionLease,
  retentionLeaseId, retentionAcquireReceiptId, retentionCapabilityRowId,
  archiveStagingReceiptId, expectedArchiveStagingReceiptDigest,
  expectedReleaseReceiptId, expectedReleased: true }`;
- `awaitExternalSupportCorrection { actionKind:
  awaitExternalSupportCorrection, supportActionId,
  correctiveInstructionDigest }`;
- `awaitExternalLockRelease { actionKind: awaitExternalLockRelease,
  lockInstructionDigest, subjects: RecoverySubjectRef[] }`;
- `awaitManualWorkingInfobaseClosure { actionKind:
  awaitManualWorkingInfobaseClosure, workingInfobaseIdentity:
  ManualWorkingInfobaseIdentity, closurePlanDigest,
  exclusiveLeaseCapabilityId }`;
- `awaitReservedOriginalClosure { actionKind: awaitReservedOriginalClosure,
  reservedOriginalIdentityDigest, exclusiveLeaseCapabilityId }`;
- `awaitExternalSupportConflictResolution { actionKind:
  awaitExternalSupportConflictResolution, supportActionId,
  supportConflictInstructionDigest }`;
- `awaitSupportRecoveryEvidence { actionKind: awaitSupportRecoveryEvidence,
  supportActionId, supportEvidenceInstructionDigest }`;
- `finalizeSupportPrerequisiteRecovery { actionKind:
  finalizeSupportPrerequisiteRecovery, supportActionId,
  finalizationPlanDigest, authorizationOutcome: cancelled |
  abandonmentFinalized }`;
- `resumeQuarantine { actionKind: resumeQuarantine, artifactId,
  quarantineId }`;
- `resumeOwnedTargetQuarantine { actionKind: resumeOwnedTargetQuarantine,
  ownedTarget: OwnedTargetLocator, quarantineId,
  expectedQuarantinedDigest }`;
- `finishArchive { actionKind: finishArchive, archiveId,
  archiveStagingReceiptId, expectedArchiveStagingReceiptDigest,
  handoffLineageDigest, retentionLeaseIds[], expectedReleases:
  HandoffRetentionReleaseReceipt[], expectedReleaseSetDigest }`; or
- `finishCleanup { actionKind: finishCleanup, archiveId,
  ownedTargets: OwnedTargetLocator[], expectedAllAbsent: true }`.

Variant fields shown above are additional to the four common fields and all
other fields are rejected. `expectedObservations` is non-empty, canonical, and
unique by `(observationKind, subject)`; `expectedPostconditionDigest ==
sha256(canonical(expectedObservations))`. `actionDigest ==
sha256(canonical(action-without-actionDigest))`. Actions contain only registered
IDs/metadata object IDs, exact object or owned-role sets, immutable
checkpoint/session bindings, and expected postconditions; they contain no
command, credential, or path. Retention lease IDs are the canonical exact set
from archived handoffs; their actions reproduce the bound capability/acquisition
receipt and can mutate only provider lease metadata.
For `preArmSupportCancellation/finalize`, every action repeats the current
`PreArmCancellationFinalizationPlan.finalizationAttemptId` and digest; its other
fields equal the exact named plan/observation/policy/receipt-plan projection.
Every effect action's `receiptRef` is the one byte-identical
`source=finalizationPlan` ref for that effect kind. Acquisition/cancellation/
release actions are present iff that ref is planned rather than already
realized by the prior operation; the update action is present exactly for
`selectiveUpdateDisposition=perform` and absent for `notRequired`,
`alreadyExact`, and `alreadyApplied`. The mode identity fields follow the
reserved/separate exclusive presence rule. `recheckPreArmCancellationFinalization`
is the sole receiptless observation action. `finishPreArmCancellationRecovery`
is always last and maps only `recoveryFinalization`; it publishes the local
terminal envelope after cancellation and both releases are proven, and never
performs or repeats any external effect.
For `finishArchive`, `expectedReleases` is canonical and unique by lease ID,
its lease projection equals `retentionLeaseIds`, and
`expectedReleaseSetDigest == sha256(canonical(expectedReleases))`.
Every `releaseRetentionLease` and `finishArchive` action repeats the exact
`ArchiveStagingReceipt.stagingReceiptId`/`receiptDigest`; `finishArchive` also
repeats the handoff lineage, while each release binds that lineage transitively
through the exact staging receipt digest. These values are bound by the
preceding successful `observeArchiveStaging` outcome. The schema
forbids release actions before that outcome or with stale/substituted staging
evidence. Every observed lease, including one already reported as `released`,
retains its exact release action: a held lease completes it as `performed`,
while an already released lease must recover the byte-identical durable receipt
as `recoveredReceipt`. In both cases the action's
`expectedReleaseReceiptId` and derived receipt digest equal its
`expectedReleases` entry. A bare released-state observation cannot prove that
receipt or authorize `finishArchive`. An unknown staging write/observation
remains archive recovery with all leases held.
`EffectReceipt` is the closed `receiptKind`-tagged `oneOf` of
`recoveryAction { receiptKind: recoveryAction, receiptId, producerActionId,
producerActionDigest, terminalObservationDigests: Sha256[], receiptDigest }` or
the already defined `PreArmCancellationEffectReceipt`. In either branch
`receiptDigest == sha256(canonical(receipt-without-receiptDigest))` and the
observation list is the exact action postcondition projection described below.
`RecoveryActionOutcome` is the closed tagged `oneOf` of `performed { outcome:
performed, actionId, actionDigest, expectedPostconditionDigest,
observedPostconditionDigest, receipt: EffectReceipt,
terminalObservationDigests: Sha256[], outcomeDigest }`, `recoveredReceipt {
outcome: recoveredReceipt, actionId, actionDigest,
expectedPostconditionDigest, observedPostconditionDigest,
receipt: EffectReceipt, terminalObservationDigests: Sha256[], outcomeDigest }`, or
`alreadySatisfied {
outcome: alreadySatisfied, actionId, actionDigest,
expectedPostconditionDigest, observedPostconditionDigest,
terminalObservationDigests: Sha256[], outcomeDigest }`. In every variant the
observed postcondition equals the expected one, the observation-digest list is
non-empty and equals, in `expectedObservations` order, the
`observationDigest` fields of the enclosing `matches` observations whose
kind, subject, and expected digest match that projection one-to-one. No
unrelated, extra, missing, or reused observation may satisfy an action;
the matched observation projection is exactly `{ observationKind, subject,
expectedDigest: observedDigest }[]` in expected order, so
`observedPostconditionDigest == sha256(canonical(the-matched-projection))` and
equals the expected digest. `outcomeDigest ==
sha256(canonical(outcome-without-outcomeDigest))`. A completed recovery has one
outcome for every ordered action, in the same order; failed/unknown actions
cannot appear in completed data and leave the recovery plan current instead.
Every receipt's `producerActionId`/`producerActionDigest` equals its outcome's
`actionId`/`actionDigest`. For a pre-arm effect action, receipt kind is exactly
`preArmCancellationEffect`; its receipt ID/kind/intent equal the action's
planned ref and its realized form is the attempt progress's byte-identical
effect receipt. Every other mutating action uses `recoveryAction`.
`alreadySatisfied` is legal only for observation-only actions
`verifyTaskFingerprint`, `observeCommit`,
`observePreArmCancellationOutcome`,
`recheckPreArmCancellationFinalization`,
`observeSupportPrerequisiteHistory`, `observeWorkingInfobaseLease`,
`observeReservedOriginalLease`, `observeRetentionLease`, and
`observeArchiveStaging`. `finishPreArmCancellationRecovery` always publishes
the durable local recovery terminal envelope and receipt; it therefore requires
`performed` or `recoveredReceipt`, never receiptless `alreadySatisfied`. Every mutating
action requires `performed` for a newly observed effect or `recoveredReceipt`
when response-loss reconciliation discovers its previously persisted receipt.
For each ordinary lease-release action, that receipt ID equals the action's
`expectedReleaseReceiptId`, its action fields and observation list equal the
outcome byte-for-byte, and its digest is reproduced in the archive mapping; a
receiptless already-satisfied lease release is
invalid. `finishArchive` outcomes and `ArchiveData` carry the same canonical
lease-to-release-receipt set, so exact-once release cannot be inferred from a
bare released-state observation.
Release receipt IDs are allocated in the immutable plan. Because their receipt
digests cover only the already-bound action digest and deterministic successful
observation projection, the expected lease-to-receipt mappings are derivable
before effect intent; if any observed field differs, the action is not terminal
and no `finishArchive` action runs.

The closed action schema enforces this action-kind projection; substitutions
are invalid even when their digests happen to match:

| Action kind | Required expected-observation projection |
| --- | --- |
| `releaseOwnedLocks` | one `lockOwnership` observation for every exact subject, all proving the empty owned-lock destination |
| `restoreOriginal` | one `objectFingerprint` observation for the bound `reservedOriginalInfobase` and checkpoint fingerprint |
| `restoreTaskCheckpoint`, `recreateTaskInfobase`, `verifyTaskFingerprint` | one `taskFingerprint` observation for the registered task IB and exact expected fingerprint; a plan selects one of these actions for a given terminal observation |
| `observeCommit` | the exact `repositoryVersion`/integration-set observation named by the action |
| `observePreArmCancellationOutcome` | the exact root-lock, mode-lease, original fingerprint, support graph, and authorization-state observations needed to produce one complete `PreArmCancellationEffectObservation`; its terminal-effect receipt IDs come only from the bound durable operation journal, and no partial observation can publish the finalize plan |
| `acquirePreArmRootGuard` | exactly one `lockOwnership` observation proving the bound configuration root held by this finalization attempt |
| `acquirePreArmModeLease` | exactly one mode-specific `reservedOriginalLease` or `workingInfobaseLease` observation proving the bound exclusive lease held by this attempt |
| `recheckPreArmCancellationFinalization` | one `finalizationPolicy` observation with registered subject ID equal to `finalizationAttemptId` and expected digest equal to the immutable recheck-policy digest, plus fresh root-lock, mode-lease, authorization, original-target, and support-graph matches; the full actual partition is retained in `PreArmCancellationFinalizationRecheckEvidence`, so an allowed predicate-bound non-root tail need not equal the old partition digest. It creates no effect receipt |
| `applyPreArmCancellationSelectiveUpdate` | one `objectFingerprint` observation per exact target in the selective plan, while both guards remain held |
| `persistPreArmSupportCancellation` | exactly one fresh `supportActionAuthorization=cancelled` observation for the frozen no-arming action; it does not update or release anything |
| `releasePreArmModeLease` | exactly one mode-specific lease observation proving release by this attempt |
| `releasePreArmRootGuard` | exactly one root `lockOwnership` observation proving the attempt owns no root lock |
| `finishPreArmCancellationRecovery` | fresh authorization-cancelled, selected original-target, support-graph, mode-lease-released, and root-unlocked observations bound by the complete receipt plan; it never observes or creates an arming receipt or repeats an external effect |
| `quarantineArtifact`, `resumeQuarantine` | exact `artifactPresence` plus `quarantinePresence` observations for the named artifact/quarantine |
| `resumeOwnedTargetQuarantine` | one `quarantinePresence` observation for the exact `ownedRole` subject proving its named quarantine state; no artifact ID is legal |
| `observeSupportPrerequisiteHistory` | the exact anchor and one ordered `repositoryVersion` observation per entry in the bound contiguous partition |
| `updateOriginalSelectedTargets` | one `objectFingerprint` observation per root/metadata target in the bound selective plan |
| working-IB/reserved-original/retention lease observe or release | exactly one matching lease-kind observation for the exact external-IB/retention-lease subject and capability-bound destination |
| `observeArchiveStaging` | exactly one `archiveStagingPresence` observation whose subject/expected digest equal the named durable staging receipt |
| each `awaitExternal*`/`awaitSupportRecoveryEvidence` action | only the exact repository-version, support-graph, lock, IB, or retained-artifact observations named by its immutable instruction digest |
| `finalizeSupportPrerequisiteRecovery` | exact `supportActionAuthorization` terminal state plus destination `supportGraph` observations |
| `finishArchive` | exactly one `archivePresence` observation; its staging-receipt and release-set digests bind the preceding durable staging and lease-release outcome receipts one-to-one, so their observations are not reused |
| `finishCleanup` | one fresh exact `quarantinePresence`/owned-role observation proving `absent` for every owned target after deletion; a merely quarantined target cannot complete cleanup and no earlier resume observation is reused |

The schema snapshot encodes these as action-discriminated observation tuples;
an action cannot carry the observation projection of another row.
`RecoveryUnknown` is the closed
`{ observationKind: RecoveryObservationKind, subject: RecoverySubjectRef,
expectedDigest }`; `remainingUnknowns` contains its exact canonical
projection of `unknown` observations and no match/difference. `recoveryDigest` covers the prior
operation, target/effect class, planned result phase, canonical observations,
exact ordered actions, remaining unknowns, and their anchors.
All armed-support-specific fields are absent for every other target. For
`target: preArmSupportCancellation`, `preArmCancellationStage` is required and
the authorization is byte-identically `frozenForRecovery` with `freezeKind:
preArmCancellationEffect`, no arming receipt, and the same pending action/
cancellation digests as the interrupted operation. In stage `observeOutcome`,
all four pre-arm observation/finalization/blocker fields are absent and the sole observation
action records typed unknowns without changing phase or authorization. In stage
`finalize`, the observation, finalization plan, and progress are required,
progress starts as `notStarted`, and
their action/operation/digest/mode values
match byte-for-byte, `effectObservationDigest` equals the observation record,
and the finalization plan is derived solely from that conclusive record plus the
previously approved cancellation preview. The armed-support history,
disposition, late-phase, corrective-action, distribution, finalization-plan,
guard-proof, and working-IB-closure fields are absent in both stages.
`preArmCancellationKnownBlocker` is present exactly when the current fresh
finalize plan was published by `preArmCancellationRecoveryBlocked` and remains
current until that plan completes or is replaced. Its
`compensatedAttemptAuditDigest` equals the newly appended last
`priorAttemptAudits` entry, its `previousRecoveryDigest` equals the blocked
approved request, and its full evidence/instruction is byte-identical to the
typed stop. It is absent for outcome-observed and recheck-replanned finalize
plans. The generic armed-support `requiredExternalAction` remains absent; this
dedicated closed blocker is the durable pre-arm status instruction. The
pre-arm plan's `recoveryDigest` covers its stage, complete observation when
present, immutable finalization plan when present, generic observations/actions,
prior compensated-attempt audits, the complete known blocker when present, and
remaining unknowns. Mutable current-
attempt progress is instead covered by the current operation/journal record and
every realized immutable receipt; changing it never changes the already-
approved plan. Schema tests reject an arming receipt, an armed-support
field, a phase not bound by the cancellation preview, or any attempt to move
directly from unknown observation to finalization without the distinct approved
digest.

For a frozen armed support authorization, both support-history cursors,
`supportHistoryPartition`, `supportVersionObservations`,
`supportRecoveryDisposition`, `supportRecoveryFinalizationPlan`, and
`supportLateRelevantResultPhase` are required;
`supportHistoryFromCursor` equals the frozen authorization's
`expectedBeforeHistoryCursor`, as does
`supportRecoveryFinalizationPlan.historyFromCursor`; the partition endpoints
equal the two support cursors, and its entries map one-to-one in
repository order to the observations with identical classification and
canonical semantic-delta digest. `supportVersionObservationDigest ==
sha256(canonical(supportVersionObservations))`; the status handle, terminal
recovery result, recovery receipt, and archive lineage use that byte-identical
digest for this exact ordered list. Its arming prefix equals the immutable arming
receipt partition; the first root/support observation after the receipt cursor
is the action version or the exact invalid/conflict observation that froze it;
for `restoreThenReauthorize`, `plannedResultPhase` equals the authorization's
`cancelledPhase` and the late-relevant field equals its bound
`relevantAdvancePhase`; for `preserveExternalAndReauthorize`, both equal the
bound `relevantAdvancePhase`; for `restoreThenAbandon`, both are the literal
`abandonmentReady`. The observations cover their complete contiguous range and may be empty
only for a versionless `originalNotClean` recovery whose ordinary observations
contain the exact expected/observed original fingerprints. The forbidden-success literal is
required exactly for `restoreThenAbandon`. `requiredExternalAction` is paired
with exactly one matching await action: support correction when graph/content is
not at the destination, lock release for a known blocked guard target,
working-IB cleanup/closure in separate mode, reserved-original Designer closure,
external-support conflict resolution, or missing recovery evidence. It is
absent only when the final
state is exact and no known external blocker remains. `recoveryDigest` covers every present support field; a lost stop
response therefore never reduces corrective work to hashes or prose.
The finalization plan is always present, is covered by `recoveryDigest`, and is
the authoritative lock/update target even when no corrective instruction is
present. `latestSupportRecoveryGuardProof` is present iff locking has been
attempted for the current finalization `planDigest` and that current-plan
attempt ended in a blocked/stopped verified release with
`authorizationOutcome: unchanged`; a terminal completed proof/receipt replaces
the plan. Its `finalizationPlanDigest` equals
the current finalization `planDigest`; observing any new history/recomputing the
plan moves the old proof to immutable audit history and removes it from
`latestSupportRecoveryGuardProof`. A correction/conflict/reapproval stop may
instead expose that released old attempt only as
`priorSupportRecoveryGuardProof`, byte-for-byte from immutable audit history and
absent from the recomputed plan's `latest` field. Current-plan lock-blocked and
working-IB/reserved-original closure stops expose
`supportRecoveryGuardProof` byte-for-byte equal to the
nested latest proof.
The working-IB closure plan is required exactly for
`separateWorkingInfobase` recovery and absent in reserved mode. It is `desired`
while a corrective/conflict version is still future, becomes `materialized`
only from complete observed history, and must be materialized before any
terminal guard attempt. That state/digest transition is covered by
`recoveryDigest`, clears old approval/latest proof, and yields
`supportRecoveryReapprovalRequired` before effect.
`repository.status.recovery` uses this
same type. Therefore a client that lost the original stop response can inspect
the exact predetermined recovery effects before approving the compact handle.
For an interrupted task deployment, task `merge.apply`, or compatible general
operation classified as an `authoritativeTaskConfigurationMutation`, recovery
may only restore the bound checkpoint or recreate the owned File IB and prove
its fingerprint; it never treats an unknown task mutation as successful or
blindly replays it.

A `supportPrerequisite` recovery plan is reconcile-only. With
`restoreThenReauthorize`, its destination retains every semantically partitioned
valid routine, proven disjoint external-support, and
disposition-preserved externally owned version in repository order,
and inverses only this authorized action's invalid content/support deltas
relative to that preserved baseline. Its `plannedResultPhase` is the authorization's bound
`cancelledPhase`, and terminal recovery cancels rather than consumes the invalid
authorization. A new manual action may then be created by fresh preflight.
With `preserveExternalAndReauthorize`, the complete proven external version is
kept as baseline, no corrective inversion is generated, the authorization is
cancelled, and recovery returns only to the relevant-advance phase for a fresh
distribution and support preflight.
An action-attributed `unauthorizedContentChanged` or `offSupportObserved`
instead forces
`restoreThenAbandon`, sets
`plannedResultPhase: abandonmentReady`, and permanently removes successful
integration as an allowed outcome. Its recovery destination retains the same
ordered routine and proven externally owned support baseline, then removes
only this action's invalid deltas and task-only editability; abandonment cleanup
never removes another actor's support state. Its ordered actions observe every external
root version since the authorization cursor, verify any human corrective
versions, prove the final support graph equals the exact disposition-bound
destination, selectively update only the finalization plan's locked original
targets if needed, prove all
root/task locks absent, persist receipts for the complete immutable version
chain, and cancel or abandon-finalize the frozen authorization according to the
disposition. When correction is
needed, `requiredExternalAction` is the full `SupportCorrectiveInstruction` and
the ordered actions pause at `awaitExternalSupportCorrection`; Unica cannot
create that repository version itself. Until those external facts exist they
remain typed unknowns and `repository.recover` returns
`supportCorrectionPending` rather than publishing success. The retained chain,
including every invalid/corrective version and recovery disposition, is
archived even when the task is later abandoned.
Terminal recovery selects exactly `supportLateRelevantResultPhase` when either
the approved history partition or the post-release partition contains a relevant
routine or external-support version; otherwise it selects
`plannedResultPhase` only when the post-release tail is entirely
`unrelatedRoutine`. The post-release partition ends immediately before any
authorized/invalid/corrective/unattributed support successor. Finalization and
its receipt remain terminal; that successor is persisted as
`DeferredRepositoryAdvance`, selects `supportLateRelevantResultPhase`, and is
classified by the next exact routine update from the receipt cursor. It never
reopens the frozen authorization or recomputes a plan whose authorization has
already been cancelled/abandon-finalized. Both permitted choices, the complete
allowed partition, and optional deferred observation are covered by the
terminal result/receipt digests; no runtime branch may invent a phase.

At most one recovery plan is current. While it is current, read-only status is
allowed but every other mutating/authoritative call is rejected with
`recoveryPlanPending`; only the exact `repository.recover` apply may execute it.
A no-effect abandonment-preview plan may instead be cancelled by the exact
recover cancellation variant below. Cancellation atomically invalidates the
plan before normal main verification/commit can resume; an effect/recovery plan
that has entered `recoveryRequired` is never cancellable.

`recentOperations` contains bounded `{ operationId, operation:
TaskOperationSelector, terminalKind, resultDigest }` records. `terminalKind` is
the closed enum `completed`, `stopped`, or `rejected` and is the exact
projection of the durable terminal envelope's `resultKind`; it is not a second
caller-selected classification. Together the
current phase and tagged handles provide
every ID/digest required by the next legal request after a response is lost;
callers never reconstruct IDs or paths. Status does not write observations or
reconcile a journal.

### `unica.branched.archive` — `previewedJournaledEffect`

Request fields: `taskId`, `operationId`, `outcome` (`success` or `abandoned`),
`reason` (required and non-empty for `abandoned`), and `dryRun?: true` for
preview; `reason` is forbidden for `success`. Apply requires `dryRun: false`
and `approvedPreviewDigest`.
`PreArmCancellationArchiveEntry` is the closed `{
supportActionId, effectObservation: PreArmCancellationEffectObservation,
finalizationPlan: PreArmCancellationFinalizationPlan,
finalizationPlanDigest, receiptPlanDigest,
finalizationRecheckEvidence:
PreArmCancellationFinalizationRecheckEvidence,
completedFinalizationProgress:
PreArmCancellationFinalizationAttemptProgress,
finalizationAttemptAuditDigest, supportCancellationReceiptId,
supportCancellationReceiptDigest, preArmRecoveryReceiptId,
preArmRecoveryReceiptDigest, recoveryReceiptDigest,
selectiveUpdateProof: SelectiveRepositoryUpdateProof,
postReleaseObservedHistoryCursor: RepositoryHistoryCursor,
postApplyHistoryPartition: RepositoryHistoryPartition,
deferredRepositoryAdvance?: DeferredRepositoryAdvance, resultingPhase,
entryDigest }`. Its field mapping is exact:
`status.priorSupportActionId == supportActionId`; the full effect observation,
immutable finalization plan, completed progress/full receipts,
plan/receipt/recheck/attempt digests, both receipt ID/digest pairs, and recovery
digest equal the terminal recovery and status projections field-by-field. The
plan retains every prior compensated-attempt audit; its two nested digests equal
the compact fields. The progress is the `completed` branch, has the same attempt
ID and recheck evidence, and its `attemptAuditDigest` equals the compact audit
digest. The
full selective proof's digest, post-release cursor, post-apply partition digest,
optional deferred-advance digest/presence, and result phase equal the compact
status fields and full terminal recovery; none can be omitted or substituted.
`entryDigest ==
sha256(canonical(entry-without-entryDigest))`.
Success
requires `committedAndUnlocked`; abandonment requires no
worker, lock, original difference, or unknown effect. Preview `data` is
`ArchivePreviewData { outcome, retainedEntryNames[], excludedRoles[],
eligibilityDigest, previewDigest }`; applied `data` is `ArchiveData { archiveId,
outcome, schemaVersion, sha256, archiveStagingReceipt: ArchiveStagingReceipt,
retainedEntryNames[],
supportArmingReceiptIds[], supportPrerequisiteReceiptIds[], supportCancellationReceiptIds[],
supportRecoveryReceiptIds[], preArmRecoveryReceiptIds[], preArmCancellationRecoveries:
PreArmCancellationArchiveEntry[], handoffRetentionReleases:
HandoffRetentionReleaseReceipt[], deferredAdvanceConsumptionReceiptIds[],
previewDigest }`, where `HandoffRetentionReleaseReceipt` is the closed
`{ retentionLeaseId, releaseActionId, releaseActionDigest, releaseReceiptId,
releaseReceiptDigest }` record.
All five support-receipt lists and the pre-arm recovery list may be empty. Each
pre-arm entry's cancellation receipt ID appears exactly once in
`supportCancellationReceiptIds`, its distinct recovery receipt ID appears
exactly once in `preArmRecoveryReceiptIds` (never in the armed-support
`supportRecoveryReceiptIds`), and every terminal pre-arm recovery has
exactly one entry. When non-empty the archive retains
each arming boundary and each external
normal root-only prerequisite, explicit cancellation (including preserved
external-support state), or exceptional invalid/corrective recovery
version/actor, prior gate/action digest, complete version observations,
disposition, selective-update proof, and exact support/content restoration
separately from any final task-content commit.
Every task-bound external-profile retention lease is released exactly once only
after the archive durably retains its handoff/evidence lineage. Release drops
the lease/reference and records its receipt; Unica never deletes the profile-
owned CF. Missing/ambiguous lease release makes archive effect unknown and blocks
cleanup rather than guessing.
`handoffRetentionReleases` is canonical, unique by lease ID, and maps one-to-one
to archived handoffs and the corresponding `releaseRetentionLease` action
outcomes; missing, duplicate, swapped, or receipt-digest-mismatched entries are
invalid. `deferredAdvanceConsumptionReceiptIds` retains every consumption
receipt referenced by an archived terminal support lineage.
Apply first writes/fsyncs the complete handoff/evidence lineage and persists
`archiveStagingReceipt`; only after its digest and live presence are observed
may any retention lease be released. It then records exact release receipts and
atomically publishes the final archive. Crash points before staging leave all
leases held; after staging they use the archive recovery grammar above and may
release only against that exact receipt. A stale/missing staging receipt blocks
release and final publication.
Immediately before any retention-lease release, archive canonicalizes every
frozen provider root and exact recovery source again and checks each against
every owned, quarantine, archive-staging, and other destructive target in both
containment directions. Provider roots and sources are never removable roles.
Any identity, canonicalization, or overlap mismatch returns `unsafeTaskPath`,
releases no retention lease, and performs no move/quarantine/deletion.

A non-terminal or frozen support-action authorization is not abandonment-safe.
An awaiting/armed authorization is rejected with
`supportPrerequisiteReconciliationRequired` until the caller completes its
exact arm/reconciliation path or the explicit
`repository.update(mode="supportPrerequisiteCancellation")` flow. A frozen
authorization always requires its exact recovery plan. If accepted prerequisite
receipts leave any task-only support transition in force and no successful
task-content commit consumes that need, abandonment preview returns an
inverse-only `SupportCleanupProposalData { originPhase,
supportPrerequisiteReceiptIds[], currentSupportGraphDigest,
requiredRestoreTransitions: SupportTransition[], proposalDigest }` and stops
with `manualSupportCleanupRequired` plus `SupportCleanupPreviewStopData`. The
preview persists only its operation/evidence and proposal digest: it publishes
no support authorization, acquires no external lease, and leaves phase/state
unchanged. `proposalDigest ==
sha256(canonical(proposal-without-proposalDigest))`; the enclosing
`previewDigest` covers that exact proposal plus archive eligibility anchors, and
the approved apply binds both without accepting caller-reconstructed fields.
The distinct approved archive apply rechecks the proposal, journals
intent before every external lease effect, runs the recovery-handoff and
mode-specific baseline gates, and only then atomically publishes
`SupportActionAuthorizationData { purpose: abandonmentCleanup, state:
awaitingArm, ... }`; it stops with `SupportCleanupStopData`. A lost apply
response is reconstructed from status and the operation record. The user first
acquires/arms the root, then commits exactly those root-only restore transitions,
and reconciles them through the normal typed
prerequisite path, and must leave a clean repository-equal original before
`archivedAbandoned`. This is the reachable cleanup edge from any otherwise
abandonment-safe pre-terminal phase; archive never silently retains task-only
editability. A cleanup authorization contains only
`restoreConfigurationChangesDisabled`/`restoreObjectLocked` transitions derived
from accepted task receipts and current semantic support state; it cannot make
another object more editable.
Before publishing that cleanup authorization, the same root-reachable recovery-
distribution/handoff and mode-specific separate-IB authorization-baseline gates
apply during the approved apply, never during `dryRun: true`. Missing/stale
evidence or a capability-proven busy/dirty baseline returns the typed archive
`supportPreflightInconclusive` stop, creates no authorization or archive, and
preserves the origin phase after verified lease release. An unknown acquire,
inspection, or release effect is recovery-bound rather than a retryable stop.

Successful cleanup-prerequisite reconciliation enters `abandonmentReady`; from
that phase only read-only status, typed `repository.update(mode="routine")`,
and `branched.archive(outcome="abandoned")` preview/apply are normally legal. A
new archive/routine reclassification may create another awaiting cleanup
authorization; while awaiting/armed, only its exact arm/prerequisite
reconciliation/cancellation is additionally legal, and if frozen only status/
recovery remains. The archive apply rechecks the complete receipt chain, clean
repository-equal original, zero owned/unknown effects, and zero task-only
support transitions before publishing `archivedAbandoned`.

An abandonment preview from `locked` is `rejected` with
`taskAbandonmentNotSafe` and
requires `repository.unlock(reason="abandonment")`; verified full release with
the original unchanged returns `synchronized`. From `mainMerged` or
`mainValidated`, the preview instead stops with
`abandonmentRecoveryRequired` and publishes a digest-bound plan that restores
the original rollback checkpoint, verifies the before fingerprints, releases
the complete lock set, and has `plannedResultPhase: synchronized`. The preview
performs no external effect and does not change phase. `repository.recover`
must execute that exact approved plan before a new abandonment preview can
become eligible. An active/unknown operation or absent rollback proof cannot
produce this plan and remains unsafe/recovery-bound.

### `unica.branched.cleanup` — `previewedJournaledEffect`

Request fields: `taskId`, `operationId`, `archiveId`, and `dryRun?: true` for
preview; apply requires `dryRun: false` and `approvedPreviewDigest`. Preview
`data` is `CleanupPreviewData { archiveId,
outcome, removableRoles[], ownedTargetLocator: OwnedTargetLocator, markerDigest,
previewDigest }`;
applied `data` is `CleanupData { quarantineId, outcome, removedRoles[],
retainedArchiveId, markerDigest, absentObservationDigests: Sha256[],
cleanupReceipt: CleanupReceipt, previewDigest }`, where `CleanupReceipt` is the
closed `{ cleanupReceiptId, operationId, archiveId, approvedPreviewDigest,
ownedTargets: OwnedTargetLocator[], quarantineId, absentObservationDigests:
Sha256[], resultingPhase: cleanedSuccess | cleanedAbandoned, receiptDigest }`
and `receiptDigest == sha256(canonical(receipt-without-receiptDigest))`.
Preview and apply both rerun
every path/marker/reparse/Git/root guard. They compare every archived frozen
canonical retention-provider-root/source boundary with every live-canonical
owned, quarantine, and proposed destructive target in both containment
directions. If a provider root or source still exists after its verified
archive-time lease release, cleanup also canonicalizes it live and requires the
same non-overlap; a changed live identity, resolution failure, equality,
ancestor, or descendant overlap returns `unsafeTaskPath`. Absence after the
verified release is admissible: the external owner may move/delete its source,
and cleanup still uses the frozen boundary without inventing or touching the
missing object. A provider root or source is never a removable role. Preview
proposes nothing and apply performs no quarantine/deletion on an unsafe result.
Apply writes intent, quarantines on the same filesystem,
and journals each role deletion so restart can reconcile a missing,
quarantined, partially removed, or fully removed owned instance. It never
writes, moves, quarantines, or deletes provider-owned recovery content.
Completed apply requires one fresh exact absent observation per owned target;
the result's roles/observations equal the receipt projections, its operation/
archive/preview/quarantine fields equal the enclosing request/result, and its
phase matches archive outcome. Missing, duplicate, reused-quarantine, or
substituted absence evidence cannot publish `cleaned*`.

## Delivery Tools

### `unica.delivery.inspect` — `readOnly`

Request: no fields beyond common `cwd`/`taskId`. `data` is `DeliveryInspectionData { configurationIdentity,
repositoryIdentity, bindingMatches, mainEqualsRepository,
mainEqualsDatabaseConfiguration, platformVersion, compatibilityMode,
deliveryPermissions { distributionAllowed, updateAllowed },
distributionRuleCounts, supportLayers[], localDifferences[], warningsAreErrors:
true, statusDigest }`. `configurationIdentity` is the exact
`ConfigurationIdentity` record and both delivery permissions are explicit
booleans. Platform warnings make inspection/create fail; there is no caller
switch to weaken this policy. Secret or raw connection fields are absent.

### `unica.delivery.create` — `previewedJournaledEffect`

Request fields: `taskId`, `operationId`, `role`
(`baselineDistribution` or `refreshDistribution`), `inspectionDigest`,
and `dryRun?: true` for preview; apply requires `dryRun: false` and
`approvedPreviewDigest`. A baseline apply records
`preflightPassed` only after revalidating that exact fresh clean inspection;
a refresh preview/apply is legal exactly from `localVerified`,
`synchronizationPrepared`, `synchronizationConflicts`, `synchronized`,
`integrationPlanned`, or `blockedByForeignLock`. Each stage requires current
local-checkpoint verification, no pending/frozen support action or current
recovery plan, no other live worker, owned repository lock, original
difference, or unknown effect, and fresh inspection proving unchanged binding,
main/database configuration equal repository head, distribution permission,
and the current capability row. In `blockedByForeignLock` it additionally
requires that exact conflict operation's verified compensation and an empty
owned-lock set. Apply rechecks every gate, atomically invalidates every
Dn-and-later artifact/session/decision/verification/gate/plan/preview evidence,
and returns `localVerified`; every other phase is `taskPhaseMismatch` unless a
safety blocker has higher precedence. It does not claim the foreign lock
disappeared; the later single bounded lock attempt observes that fact without
polling.
Source is always the proven clean original; ordinary `make`/`DumpCfg` and CFU
cannot satisfy the call. Preview `data` is `DistributionPreviewData { role,
configurationIdentity, repositoryAnchor, platformVersion, inspectionDigest,
plannedArtifactKind: configurationDistribution, previewDigest }`. Applied
`data` is `DistributionData { artifactId, role, kind, sha256,
configurationIdentity, repositoryAnchor, platformVersion, createdAt,
previewDigest }`. Preview never invents an artifact ID, output hash, or creation
time.

### `unica.delivery.verify` — `contained`

Request: `taskId`, `operationId`, `artifactId`, and optional `expectedKind:
AcceptedArtifactKind`.
Only a registered task artifact can be selected. `data` is
`ArtifactVerificationData { verificationId, artifactId, kind, expectedKind?,
expectationMatched, sha256, probeId, supportIdentity?, currentEqualsVendor?,
diagnosticsDigest }` only for a completed accepted-kind result. The call stops
with `artifactKindMismatch` and the separate
`ArtifactClassificationStopData` above whenever an explicit expectation
differs or the observed kind is `configurationUpdate`/`invalidArtifact`, even
when `expectedKind` was omitted. In the latter case `expectedKind` stays absent
and `expectationMatched` is false; classification evidence is completed, but no
completed workflow result/selectable verification handle is published. With no
expectation, only an accepted distribution or ordinary-configuration kind may
complete. A probe is destroyed only after its result is durably observed;
extension-only classification is forbidden.
`classificationDigest ==
sha256(canonical(stop-data-without-classificationDigest))`. A stopped
classification never allocates a verification ID and status keeps the
artifact's optional `verificationId` absent, so recognizing CFU cannot make it
a workflow input.

### `unica.delivery.deploy` — `previewedJournaledEffect`

Request fields: `taskId`, `operationId`, verified `distributionId`, and
`dryRun?: true` for preview; apply requires `dryRun: false` and
`approvedPreviewDigest`. The artifact role must be
`baselineDistribution`; refresh distributions are merge inputs, not deployment
baselines. Preview `data` is `DeploymentPreviewData { distributionId,
distributionSha256, destinationKind: ownedTaskInstance, plannedRoles[],
previewDigest }`; applied `data` is `DeploymentData { taskInfobaseId,
taskWorkspaceId, distributionId, vendorIdentity, currentFingerprint,
vendorFingerprint, currentEqualsVendor, sourceFingerprint, previewDigest }`.
Preview never allocates or reports task-infobase/workspace IDs or post-deploy
fingerprints. The task IB is always local File; deployment also creates guarded
`v8project.yaml`, local overlay, and `.v8-project.json`.

## Merge Tools

### `unica.merge.compare` — `contained`

Request fields are the common mutation fields `cwd`, `taskId`, `operationId`
plus `left`, `right`, and `scope`. Each side uses the exact heterogeneous JSON
`oneOf` of the four bare string literals `originalCurrent`, `repository`,
`taskCurrent`, and `taskVendor`, or the closed object `{ artifactId }`; the
object has no discriminator or additional property. `scope` is `projectDelta`
or `mainIntegration`. An artifact side must have a completed matching verification
and one exact kind/role combination: `configurationDistribution` with
`baselineDistribution` or `refreshDistribution` is allowed only for
`projectDelta`; `ordinaryConfiguration` with `ordinaryResult` is allowed only
for `mainIntegration`. `configurationUpdate`, `invalidArtifact`, an unverified
artifact, and every cross-scope role are rejected before platform dispatch, so
a classified CFU never reaches comparison/candidate/preflight logic. `data` is
`ComparisonData { comparisonId, leftAnchor,
rightAnchor, platformReportId, canonicalManifestId, deltaDigest, changeCount,
unsupportedKinds[] }`. Related configurations map by UUID; name-only mapping
fails.

### `unica.merge.prepare` — variant policy

Request is a strict closed union. `supportedUpdate` requires `checkpointId`,
an `incomingDistributionId` whose completed matching verification proves kind
`configurationDistribution` and role `refreshDistribution`, and a project-delta
`comparisonId`. Every other kind/role, including a verified CFU or ordinary CF,
is rejected as `artifactNotDistribution` before sandbox creation. A
replacement subvariant used only from `synchronizationConflicts` additionally
requires `replacesSessionId`, `expectedReplacedBaseSessionDigest`, and
`expectedReplacedDecisionSetDigest`. Both subvariants retain the literal
`mode: supportedUpdate`: the replacement triple is either entirely present or
entirely absent, and every partial combination is a request-schema error. No
second discriminator is accepted. All three fields are absent for a first prepare.
Its checkpoint/distribution/comparison must exactly match the replaced
session's immutable inputs. It builds the fresh sandbox/session first, then
atomically invalidates the old resolution workspace, decisions, and every
receipt/status handle from that generation; failure leaves the old session
current.
`mainIntegration` requires the synchronized `checkpointId`, `verificationId`,
`expectedVerificationDigest`, and `expectedRepositoryStatusDigest`; it internally creates/registers and
classifies the ordinary result CF before comparing it with a repository-fresh
original snapshot. It derives the support candidate set from the platform
report plus canonical UUID/property delta, ownership, add/delete, and reference
closure; XML paths are audit/optimization inputs only and CFU is rejected. It
then runs the exact sandbox merge without `-force` and publishes
`SupportPreflightData`.

`supportedUpdate` and its replacement/resolved-replay variants are
`contained`. `mainIntegration` is `journaledEffect`: its request's exact
verification/status digests are the guard, and it writes intent before the
first external retention-provider or mode-specific working-infobase lease
effect. The contained sandbox work remains disposable, but a manual
authorization is published only after every required lease acquisition,
inspection, and release/retention postcondition is durably observed. An
unknown external lease effect enters typed recovery and cannot be retried as a
contained preparation. The policy is selected from the closed prepare-mode
variant before support outcome classification.

A non-`ready` outcome returns the corresponding evidence-bearing stopped
variant and no session. The session schema for supported update/replay and the
nested main session is `MergeSessionData { sessionId, mode,
checkpointId, incomingDistributionId?, immutableInputHashes, anchorDigest,
settingsDigest, ordinaryResultArtifactId?, comparisonId, resultDigest?,
conflictCount, mergeResolutionWorkspaceId?, baseSessionDigest,
decisionSetDigest, resolvedSessionDigest?, supportGateId?,
supportGateDigest?, supportGateHistoryEvidenceDigest? }`. `incomingDistributionId` follows
the same mode presence rule as the status handle.
Completed `mainIntegration` data is `MainIntegrationPreparationData {
preflight: SupportPreflightData, session: MergeSessionData }`; its preflight
outcome is the literal `ready`. This makes all four support outcomes explicit
in the terminal response while preserving the typed session handle.
`baseSessionDigest` never changes for that prepared workspace;
`decisionSetDigest` starts from the canonical all-`undecided` conflict-state
map and evolves both after a decision and when a changed resolution receipt
moves a current head to `replacementPending`; immutable historical decision
records are not the active map;
`resolvedSessionDigest` exists only when all required decisions have been
materialized and zero conflicts remain. A resolution workspace ID appears only
for supported-update conflicts where manual/combine work is allowed.
`mainIntegration` requires all three support-gate/history fields,
`conflictCount: 0`, and a
resolved digest. A
  relevant-baseline digest change or any intervening relevant/support history
entry (including net-zero change-then-revert) stops before session publication,
invalidates D1-and-later evidence, and returns to `localVerified` for a fresh distribution. A
target editability restriction is classified only by support preflight. A merge
conflict, unexpected scope, support-isolation violation, or extra repair stops
with `mainPreparationMismatch`, immutable comparison/difference evidence, and
`validationFailed` for task repair. None of these outcomes enters the
synchronization conflict state. Every preparation restores a fresh sandbox;
authoritative IBs are unchanged.

`MainPreparationMismatchKind` is the closed enum
`relevantBaselineChanged`, `conflict`, `unexpectedScope`,
`supportIsolationViolation`, or `extraRepair`.
`RelevantBaselineChangedStopData` and `MainPreparationMismatchStopData` are
distinct closed schemas. Their `mismatchKinds` lists are non-empty and contain
only that enum; the former requires the singleton list
`[relevantBaselineChanged]` plus its complete partition/relevant-entry fields,
while the latter excludes that value and rejects those history-only fields.

Before original apply, the latest support preflight is invalidated by any
candidate/canonical-delta, ordinary-result, support-graph,
recovery-distribution set, generated-settings, sandbox-result, capability-row,
relevant-baseline/history, or unexpected
original-fingerprint change. `supportGateDigest` covers those semantic inputs
and the relevant baseline, not the advancing global history cursor. The cursor
is retained separately for a complete contiguous scan; an advance keeps the
gate usable only when every intervening version is capability-proven outside
the candidate/reference/support closure and the endpoint-bound history evidence
is carried into the consuming operation. An original-fingerprint-only stale
classification requires `OriginalCleanRefreshProof`; local/unowned/unknown
state enters recovery. Successful
authorized original apply does not invalidate its own gate: atomically with the
merge receipt, the immutable gate becomes `consumedByOriginalMerge` and binds
that receipt plus the authorized post-merge fingerprint. It is no longer
readiness evidence, but remains the required historical ancestor for the bound
`mainIntegration` verification and task-content commit. Any later fingerprint
that differs from that authorized post-merge fingerprint invalidates the
lineage and requires recovery; it cannot be excused as the expected merge.
Outcome precedence is
`supportPreflightInconclusive` > `vendorForbidsChanges` >
`manualSupportRequired` > `ready`. A sandbox diagnostic that cannot prove the
complete blocked subset therefore cannot report one of the more permissive
outcomes.

For a `manualSupportRequired` result, invalidating the readiness observation on
the expected arming/manual anchor advances does not erase its separately
persisted `SupportActionAuthorizationData`. That authorization is bound to the
old anchor and exact allowed delta and is usable only to arm/classify/reconcile/
cancel. It cannot resurrect the old session/plan lineage and is consumed only
after successful armed prerequisite reconciliation.

`resolvedReplay` requires the prior supported-update `sessionId`,
`expectedBaseSessionDigest`, and `expectedDecisionSetDigest`. The server-owned
CAS journal retains every immutable decision revision, while its digest-bound
current-head map supplies exactly one replay decision per conflict in canonical
conflict order; callers cannot select historical revisions, omit, duplicate,
reorder, or inject IDs. A decision with `current: false`, including one
superseded by a later resolution edit, is never replayed even though its
decision body and consumed receipt remain auditable. Replay rejects incomplete conflict
coverage and every currently selectable changed receipt not consumed by an
exact decision. Superseded and no-change receipts are immutable audit/replay
records but are not unbound changes. It then
restores the original checkpoint, replays that journal, and creates a new
immutable session. Zero remaining conflicts produces `resolvedSessionDigest`
and returns to `synchronizationPrepared`; otherwise the new session remains
`synchronizationConflicts`. This operation, not `merge.resolve`, owns sandbox
recreation and the conflict-state transition.

An incomplete/unbound replay attempt is a `stopped` observation, stays in
`synchronizationConflicts`, and performs no replay/sandbox recreation.
`ResolutionReplayStopData` returns both the exact undecided/replacement-pending
conflict IDs and exactly the
current selectable changed-receipt IDs in ascending `receiptSequence`;
`conflictDecisionsIncomplete` wins as the primary stop code while its list is
non-empty, then `unboundResolutionChanges` applies. The caller records exact
decisions or calls the digest-bound `supportedUpdate` replacement subvariant to
discard the entire old conflict workspace/session before a new operation.

### `unica.merge.conflicts` — `readOnly`

Request adds `sessionId`. `data` is `ConflictListData { sessionId,
baseSessionDigest, decisionSetDigest, mergeResolutionWorkspaceId?, conflicts[] }`.
Each conflict has `conflictId`, `objectId`,
display object, property path, `kind`, base/ours/theirs hashes, and allowed
resolutions, plus `decisionState`, the closed `stateKind`-tagged `oneOf` of
`undecided { stateKind: undecided }`, `current { stateKind: current,
decisionId }`, or `replacementPending { stateKind: replacementPending,
decisionId, causedByChangeReceiptId }`. It is the exact state hashed into
`decisionSetDigest`; no historical replaced head can appear as current. Kinds
are `twiceChanged`, `deleteModify`,
`addAddNameCollision`, `uuidMismatch`, `unresolvedReference`,
`supportRuleBlocked`, or `mergeSettingsRejected`; resolutions are `takeOurs`,
`takeTheirs`, `combine`, or `manual`. `allowedResolutions` is the persisted,
non-empty, duplicate-free subset for that exact conflict, in the canonical
order shown here. It is immutable conflict input covered by the session's
`baseSessionDigest`; `decisionSetDigest` continues to cover only the evolving
`undecided`/`current`/`replacementPending` state. Callers cannot supply or
reorder the list. The current specification does not define a static seven-by-four
kind matrix, so no handler may infer one from the enum names. Before
`merge.prepare`/`merge.resolve` is registered, the platform conflict classifier
must be fixed by real/fake fixtures which deterministically produce this exact
per-conflict list, including the availability of the typed resolution workspace
needed by `combine`/`manual`.

### `unica.merge.resolve` — `localJournaled`

Request is a strict `decisionKind` union. `conflict` requires `sessionId`,
`conflictId`, `resolution`, non-empty `rationale`, and
`expectedBaseSessionDigest` plus `expectedDecisionSetDigest`.
Both digests are CAS preconditions checked before any decision/receipt change.
After locating the current conflict and before inspecting or consuming a change
receipt, the server requires the requested resolution to be a member of that
conflict's persisted `allowedResolutions`. A non-member returns the rejected
`conflictResolutionNotAllowed` result with the exact persisted list and performs
no decision or receipt mutation.
For the named conflict, the server derives one predecessor from the current
closed state: the decision ID from `current` or `replacementPending`, or no ID
from `undecided`. The caller cannot supply or suppress that predecessor.
`combine`/`manual` also require `changeReceiptId`, exact
`objectId`, `propertyPath`, and `expectedResultSha256`; the receipt must come
from the session's typed merge-resolution workspace, match its immutable
base-session digest, be a `MergeResolutionChangeReceipt`, and have a current
status handle with `selectable: true`, `consumed: false`, and no
`supersededByReceiptId`. `expectedResultSha256` equals that receipt's
`afterSha256`. A no-change receipt ID, consumed/superseded receipt, stale
session/generation, or target/hash mismatch returns `changeReceiptStale` before
recording a decision and consumes nothing. Recording a `combine`/`manual`
decision atomically flips only the selected handle to `consumed: true` and
`selectable: false`; it neither supersedes nor consumes a different-target
receipt. A non-receipt `takeOurs`/`takeTheirs` decision never consumes a
selectable manual-change receipt, so resolved replay continues to stop on that
unbound receipt until it is bound by a later exact decision or the whole
session is replaced.

Every successful conflict decision creates a new immutable decision revision
and atomically makes it the conflict's sole current head. Its `data` is
`ConflictDecisionData {
decisionId, sessionId, baseSessionDigest, conflictId, resolution,
rationaleDigest, changeReceiptDigest?, replacesDecisionId?, decisionDigest,
revisedDecisionSetDigest }`. `sessionId`/`baseSessionDigest` reproduce the
approved request and immutable session. `replacesDecisionId` is present exactly
when the pre-operation conflict state was `current` or `replacementPending` and
equals that state's decision ID; it is absent for `undecided`. In particular,
when a selected changed receipt demoted the prior decision and no intervening
decision exists, the field equals that receipt's
`pendingReplacementDecisionId` and singleton `supersededDecisionIds` (or the
pending ID carried through its same-target receipt-supersession ancestry). The
old decision's derived handle receives `replacedByDecisionId` while its body
and previously consumed receipt remain unchanged; `replacementPending` is
cleared. `decisionDigest ==
sha256(canonical(data-without-decisionDigest-and-revisedDecisionSetDigest))`,
so `replacesDecisionId` is covered while the evolving set digest cannot create
a cycle. `revisedDecisionSetDigest` hashes the new current-head state.
`changeReceiptDigest` is required exactly for
`combine`/`manual`, equals the consumed receipt's canonical digest, and is
absent for `takeOurs`/`takeTheirs`; schema tests reject omission, injection, or
a digest from another resolution-workspace generation. Schema/transition tests
also reject a missing/extra/wrong `replacesDecisionId`, replacement of a
non-head historical decision, two current heads, replay of a historical head,
or any rewrite/reactivation of the predecessor's consumed receipt. The
merge-conflict status handle repeats all immutable producer fields byte-for-byte
and exposes only the derived current/supersession/replacement fields described
above.

For this `changeReceiptStale` branch, `DigestErrorContext.expectedDigest` hashes
the closed required selector state `{ mutationOutcome: changed, sessionId,
baseSessionDigest, workspaceGenerationId, affectedTarget,
expectedResultSha256, consumed: false, selectable: true,
supersededByReceiptId: null }`; `observedDigest` hashes the same field set with
the located receipt/handle values (using `null` for an absent handle). They are
therefore unequal. `producerId` is the `changeReceiptId`. The rejection context
intentionally contains no producer selector: its sole advertised action is
status, whose current merge-conflict handle repeats the receipt's immutable
producer fields for review before the caller recreates any compatible mutation.

`adaptedDelta` requires an `unexpected` synchronized-task `verificationId`,
`expectedVerificationDigest`, `canonicalDeltaDigest`,
`differenceManifestId`, `differenceDigest`, and non-empty `rationale`. Its
`data` is `AdaptedDeltaDecisionData { decisionId, verificationId,
canonicalDeltaDigest, differenceDigest, rationaleDigest,
adaptationDecisionDigest }`. A subsequent `merge.verify` bound to that decision
must reproduce the same delta/difference before it may return `adapted` and
advance to `synchronized`. Only one decision can be recorded for an exact
verification/difference digest; a different second decision is rejected until a
new verification is produced. No authoritative IB is mutated by either
decision. The adaptation status handle repeats `verificationId` and
`adaptationDecisionDigest` byte-for-byte and cannot expose a merge decision-set
digest.

### `unica.merge.apply` — `preparedJournaledEffect`

Request fields: `taskId`, `operationId`, `sessionId`, `target` (`task` or
`original`), and `approval: DigestApproval` whose digest equals the exact
`resolvedSessionDigest`.
The `original` variant additionally requires `planId`, `expectedPlanDigest`,
`integrationSetId`, `expectedIntegrationSetDigest`,
`lockSetId`, `expectedLockSetDigest`, `supportGateId`, and
`expectedSupportGateDigest`, `expectedSupportGateHistoryEvidenceDigest`.
`data` is `MergeApplyData { mergeReceiptId, sessionId, resolvedSessionDigest,
target, beforeAnchor,
afterAnchor, resultFingerprint, repositoryHistoryCursor?, supportAuditDigest,
appliedDecisionIds[],
rollbackCheckpointId?, sourcePublicationId?, sourceFingerprint?,
taskInfobaseFingerprint?, integrationSetDigest?, lockSetDigest?,
supportGateDigest?, supportGateHistoryEvidenceDigest? }`.
`appliedDecisionIds` is the canonical conflict-order projection of the
resolved session's current decision heads and excludes every superseded or
replaced historical revision; it is byte-identical to the projection used to
derive `resolvedSessionDigest` and by `resolvedReplay`.
Task apply replays update plus typed manual changes and proves D1 ancestry. It
then performs a full task-IB dump into staging, validates/builds that staged
source, atomically publishes it into the task workspace, proves canonical task
IB/XML fingerprint equality, and emits task-context cache/domain events before
success. The three publication/fingerprint fields are required for task apply
and absent for original apply; no compatible general dump/load call owns this
phase transition.
Original apply requires the exact integration/owned-lock sets and forbids
substantive repair in original. For the original variant,
`rollbackCheckpointId` is required and names the capability-proven recovery
source created and verified before mutation; `integrationSetDigest`,
`lockSetDigest`, `supportGateDigest`, and `supportGateHistoryEvidenceDigest` are
also required. `repositoryHistoryCursor` is required for original apply, equals
the consumed gate evidence's current `classifiedThroughCursor`, and is persisted
as the merge receipt cursor for all later history guards. All six fields are
absent for task apply. Conversely, `sourcePublicationId`, `sourceFingerprint`,
and `taskInfobaseFingerprint` are required only for task apply and absent for
original apply. No other optional-field combination is schema-valid.
Immediately before writing authoritative merge intent, the use case rechecks
relevant anchors, the support graph/gate, and proves that the prepared
change/reference closure plus support root guard is a subset of the acquired
lock set. A stale anchor/gate or missing lock stops before any original
mutation, retains the exact owned locks, and requires
`repository.unlock(reason="rollback")`; successful unlock returns respectively
to `localVerified` (fresh Dn required) or `synchronized` (main preparation and
plan required). Intent is durable before either authoritative IB mutation;
unknown effect requires recovery rather than replay. On proven original-apply
success, the same journal transaction marks the supplied gate
`consumedByOriginalMerge` and binds `mergeReceiptId` plus `resultFingerprint`;
post-merge verification accepts that consumed gate only with the same receipt,
session, integration set, lock set, and fingerprint lineage.

### `unica.merge.verify` — `contained`

Request is a strict scope union:

- `localCheckpoint` takes no scope-specific ID, freezes the current task
  IB/XML boundary, creates a checkpoint, and returns `valid|invalid`; successful
  data requires the new `checkpointId`;
- `synchronizedTask` requires `sessionId` and returns
  `equivalent|adapted|unexpected|invalid`; an adapted rerun additionally
  requires `adaptationDecisionId` and `expectedAdaptationDecisionDigest`.
  Those two fields are either both absent or both present. When present they
  must identify the current adaptation decision produced from the prior
  `unexpected` verification of this session; a caller cannot introduce a new
  decision/digest pair in the verification request.
  `equivalent|adapted` creates a new immutable synchronized checkpoint and
  requires `checkpointId` in success data; `unexpected|invalid` does not create
  one;
- `mainSandbox` requires the main-integration `sessionId`,
  `expectedResolvedSessionDigest`, `supportGateId`, and
  `expectedSupportGateDigest`, plus
  `expectedSupportGateHistoryEvidenceDigest`, and returns `valid|invalid`
  before lock planning;
- `mainIntegration` requires `sessionId`, `expectedResolvedSessionDigest`,
  `mergeReceiptId`, `integrationSetId`, `expectedIntegrationSetDigest`,
  `supportGateId`, `expectedSupportGateDigest`, and
  `expectedSupportGateHistoryEvidenceDigest` and returns `valid|invalid`
  after the original merge.

Omitting either adapted-decision field can produce only `equivalent`,
`unexpected`, or `invalid`. `data` is `MergeVerificationData { verificationId,
scope, outcome, canonicalDeltaDigest,
checkpointId?, validationReceiptIds[], supportAuditDigest,
selectedObjectFingerprints, differenceManifestId?, differenceDigest?,
adaptationDecisionId?, mergeReceiptId?, integrationSetDigest?,
supportGateDigest?, supportGateHistoryEvidence?: SupportGateHistoryEvidence,
verificationDigest }`. Both support-gate fields are required for both main
scopes and absent for local/synchronized-task scopes.
`mergeReceiptId` and `integrationSetDigest` are required together only for
`mainIntegration`; they are absent for `localCheckpoint`, `synchronizedTask`,
and `mainSandbox`. `checkpointId` is required only for successful
`localCheckpoint` or `synchronizedTask` outcomes described above and absent for
both main scopes. `adaptationDecisionId` is present only for `adapted`, and the
difference fields follow the exact synchronized-task unexpected/adapted rules;
all other optional fields are absent. `adapted`
requires a prior exact
adapted-delta decision and a reproduced difference; otherwise the first
non-equivalent result is `unexpected`. The use case runs configured checks
itself; caller prose or arbitrary receipt paths are rejected.

Every warning from a configured verification check is materialized as an
`invalid` outcome. Before original merge it therefore stops as
`validationFailed`; after original merge it stops as
`mainMergeValidationFailed` with the mandatory recovery plan. It never uses the
delivery-only `platformWarningRejected` code, so error precedence cannot bypass
diagnostic evidence or authoritative rollback.

An invalid `mainIntegration` observation does not roll back from this contained
verifier. It atomically enters `recoveryRequired` and publishes a
digest-bound `RecoveryPlanStatus` whose exact ordered actions restore the
original from the merge receipt's `rollbackCheckpointId`, verify the before
anchor/fingerprints, release the complete owned lock set, and then enter
`validationFailed`. Only `repository.recover` may execute that plan. Failure or
interruption keeps recovery required; direct transition to task repair is
forbidden until restoration and unlock are proven.

## Repository Tools

### `unica.repository.status` — `readOnly`

Request: no fields beyond common `cwd`/`taskId`. `data` is `RepositoryStatusData { bindingIdentity,
repositoryVersion?, originalInfobaseKind, repositoryTransport,
mainEqualsRepository, mainEqualsDatabaseConfiguration, journaledLocks[],
lastObservedConflicts[], conflictObservationCompleteness
(journalOnly|readOnlySnapshotProven),
conflictsObservedAt?, activeOperation?, recovery?, statusDigest }`. This is not a
promise of a global live-lock snapshot: without a separately proven read-only
capability it reports only journal/conflict evidence observed by prior calls.
Each `ObservedRepositoryConflict` is the closed record `{ target:
RepositoryTargetIdentity, targetDisplay: RepositoryTargetDisplay, lockedBy,
computer, infobase, lockedAt }`; the last four are explicit typed values or
`null`, never omitted or inferred. `targetDisplay` is presentation-only and
cannot replace or modify `target` in schema validation, deduplication, ordering,
or control flow.

### `unica.repository.update` — variant policy

Request is a strict tagged union. `routine` requires `expectedStatusDigest`.
`supportPrerequisiteArm` is itself a closed stage union:
`armPreview { mode: supportPrerequisiteArm, stage: preview,
expectedStatusDigest, supportActionId, expectedSupportActionDigest }` has no
`operationId`, `dryRun`, or approval field; `armApply { mode:
supportPrerequisiteArm, stage: apply, operationId, expectedStatusDigest,
supportActionId, expectedSupportActionDigest, approvedArmingDigest }` has no
`dryRun`. Both require an `awaitingArm` authorization.
Every remaining update leaf has the common mutation fields `cwd`, `taskId`, and
`operationId`; none accepts `stage`. Their exact mode-specific fields are:

- `routine`: `mode: routine`, `expectedStatusDigest`;
- `supportPrerequisite`: `mode: supportPrerequisite`,
  `expectedStatusDigest`, `supportActionId`, `expectedSupportActionDigest`,
  `expectedArmingReceiptId`, and `expectedArmingReceiptDigest`; the prior gate
  itself is expected to become stale when the external root version advances
  the repository anchor;
- `supportPrerequisiteCancellation`: `mode:
  supportPrerequisiteCancellation`, `expectedStatusDigest`, `supportActionId`,
  `expectedSupportActionDigest`, and `reason` (`taskChanged`, `abandoned`, or
  `operatorCancelled`). The `expectedArmingReceiptId` and
  `expectedArmingReceiptDigest` pair is entirely absent for `awaitingArm` and
  entirely present for `armed`; every partial pair is invalid.

Each of these three modes has the ordinary exact preview/apply union. Preview
omits `dryRun` or supplies the literal `true` and has no approval field. Apply
requires literal `dryRun: false` plus exactly `approvedUpdateDigest` for
`routine`/`supportPrerequisite`, or `approvedCancellationDigest` for
`supportPrerequisiteCancellation`. Explicit `dryRun: null`, a preview approval,
the wrong approval name, and an apply without its approval are request-schema
errors. Cancellation is the only public path for discarding an action the human
did not start; arbitrary task/archive mutations never perform an implicit
history or lock probe. Preview performs only read-only history/report/dump/
compare evidence before approval.
`supportPrerequisiteArm(stage="preview")` is `readOnly` and its apply is
`localJournaled`: it writes only the authorization/arming receipt after a final
read-only recheck and performs no repository/original effect. The preview is not
durable and is repeated after response loss; the apply result/receipt is
durable and replayable by its operation ID. `routine`,
`supportPrerequisite`, and `supportPrerequisiteCancellation` use
`previewedJournaledEffect`; their apply variants journal before the first
external guard/update/terminalization effect. The policy is selected from the
closed mode/stage request-variant discriminator and cannot be downgraded by the
caller.

Routine preview returns `RepositoryUpdatePreviewData { mode: routine,
beforeAnchor, expectedHistoryCursor: RepositoryHistoryCursor,
observedHistoryCursor: RepositoryHistoryCursor,
deferredRepositoryAdvance?: DeferredRepositoryAdvance,
deferredAdvanceResolutionDigest?: Sha256,
plannedChanges: RepositoryPlannedChange[],
plannedRelevantObjects: RepositoryTargetIdentity[],
plannedUnrelatedObjects: RepositoryTargetIdentity[],
structuralChanges: RepositoryPlannedChange[],
structuralConfirmationRequired, historyPartition: RepositoryHistoryPartition,
selectiveUpdatePlan: SelectiveRepositoryUpdatePlan, resultingPhase,
updateDigest }`. Each planned change has exact
target identity, action (`add`, `modify`, or `delete`), and relevance reason.
The expected cursor is the task's stored cursor represented by `beforeAnchor`;
the partition endpoints equal the expected/observed cursors and contain every
version in that complete range. When a `deferredRepositoryAdvance` handle is
current, both optional fields are required, this expected cursor equals its
`fromCursor`, and `updateDigest` binds the handle's `observationDigest` plus the
complete resolution. For `classified`, the first partition entry equals
`firstObservedVersion` and reproduces its classification/semantic digest. For
`unclassified`, that exact first entry remains fixed and the preview closes
every recorded semantic-evidence gap. For `coverageUnknown`, the preview first
proves complete contiguous coverage and derives the actual immediate successor.
The preview never consumes the handle. Until these facts are complete it
returns the generic deferred-advance inconclusive stop in the result table. An
`invalid` or `corrective` first entry is treated only as a newly proven external
relevant baseline: it never validates, freezes, or reopens the terminal
authorization, and forces the recorded support-late-relevant phase. Only an
approved apply may consume the handle after reproducing this partition and
performing a no-force selective refresh; no other authoritative call may skip
it.
`plannedChanges` is the canonical unique-by-target final-state projection
obtained by folding the complete range in repository order. Multiple versions
touching one target produce one entry at its last observed repository
state/version; a net-no-op target is omitted. Relevance is `relevant` when any
contributing event is relevant and `unrelated` otherwise. Add/modify entries map one-to-one to matching
`rootPresent`/`objectPresent` planned targets at the observed version/fingerprint;
delete entries map to `objectAbsent` with the deletion version. There are no
extra/missing targets. Relevant/unrelated identity lists are the exact canonical
partition of that same change list, and `structuralChanges` is exactly its
net add/delete subset relative to `beforeAnchor`, not a transient intermediate
event.
The plan's structural-required literal equals the top-level value, which is true
iff `structuralChanges` is non-empty; its conditional capability row is the
exact row approved for that list.
From `abandonmentReady`, routine is the only allowed repository-update mode
when no cleanup authorization is current;
preview must also prove the prospective support graph can be classified. Apply
updates the original, recomputes task-only transitions, and remains
`abandonmentReady`; the next archive preview either succeeds or emits a new
inverse cleanup authorization. An unclassifiable prospective graph stops before
effect, so a concurrent routine version cannot make this terminal precursor a
dead end. While that new cleanup authorization is awaiting/armed, the only
additional mutations are its exact `supportPrerequisiteArm`,
`supportPrerequisite` reconciliation, or
`supportPrerequisiteCancellation`; if frozen, only status and its exact recovery
are legal. None reopens development or successful integration.

Support-prerequisite arming is a separate local authorization transition; it
does not update the original or repository. Preview returns
`SupportPrerequisiteArmPreviewData { mode: supportPrerequisiteArm, stage: preview, purpose,
originPhase, supportActionId, supportActionDigest, supportGateId,
supportGateDigest, candidateSetDigest, expectedBeforeHistoryCursor:
RepositoryHistoryCursor, observedHistoryCursor: RepositoryHistoryCursor,
historyPartition: RepositoryHistoryPartition,
expectedRelevantBaselineDigest, observedRelevantBaselineDigest,
expectedSupportGraphDigest, observedSupportGraphDigest,
expectedRecoveryDistributionSetDigest, observedRecoveryDistributionSetDigest,
expectedOriginalFingerprint, observedOriginalFingerprint, manualTargetMode,
expectedManualActorUsername, rootLockObservation: SupportRootLockObservation,
armingDigest }`. The partition begins exactly at the authorization's
`expectedBeforeHistoryCursor`, ends at `observedHistoryCursor`, and contains
only `unrelatedRoutine`; candidate/relevant baseline, support graph, every
recovery distribution handoff/retention lease, and original fingerprint must
still equal the authorization. The complete root-lock observation must name the
bound manual actor and mode-specific infobase. No edit or commit instruction is
published by preview.
`armingDigest == sha256(canonical(preview-without-armingDigest))`; it therefore
covers the complete partition and endpoints/digest, action/gate/candidate
identities, every expected/observed relevant-baseline/support/recovery/original
pair, target mode, actor, and root-lock observation. The apply request's
`approvedArmingDigest` equals this exact digest, and completed apply returns the
same value after reproducing every covered field; no durable preview handle is
needed to reconstruct weaker approval input.

Apply requires `approvedArmingDigest`, immediately repeats that complete
history/support/handoff/original/root-owner observation, and atomically changes
only authorization state from `awaitingArm` to `armed`. Completed data is
`SupportPrerequisiteArmData { mode: supportPrerequisiteArm, stage: apply,
supportActionId, supportActionDigest,
armingReceipt: SupportActionArmingReceipt, requiredExternalAction:
ManualSupportInstruction, armingDigest }`; the receipt reproduces the approved
partition/digests/root observation, its cursor equals the preview's observed
cursor, and the instruction is reconstructed from it byte-for-byte. The
instruction requests retaining the already-held root through
edit/commit/release; terminal acceptance uses the first-version/exact-evidence
rule above and does not claim continuous observation. Reconciliation
accepts the action only when its version is the first root/support version after
that cursor. A missing/wrong root uses `manualSupportRootLockRequired` without
arming; any other exact drift uses `supportPrerequisiteArmStale`. Both preview
and apply-final-recheck stale results keep the action `awaitingArm`; after the
human releases the root, the explicit cancellation mode must publish the full
terminal proof before fresh preflight. Missing classification capability
is inconclusive. Thus a vendor/support layer cannot appear between preflight and
authorized editing without invalidating the action.

Support-prerequisite preview proves that Unica owns no automated worker/lock or
active effect, identifies the exact human repository actor/version, and
semantically partitions that version from concurrent routine changes. It
enforces the authorization's profile-bound target mode: `reservedOriginal`
requires the reserved actor/original and a clean repository-equal post-action
original, while `separateWorkingInfobase` requires the exact distinct manual
actor/working-infobase and the unchanged reserved-original fingerprint. It
accepts only the configuration root with the candidate-bound support
transitions permitted by the stopped gate. It returns
`SupportPrerequisitePreviewData { mode: supportPrerequisite, purpose,
originPhase, postReconcilePhase, supportGateId,
supportActionId, supportActionDigest, armingReceiptId,
expectedArmingReceiptDigest, armingCursor: RepositoryHistoryCursor,
supportGateDigest, repositoryVersion, repositoryActor:
RepositoryActorIdentity, authorizedTransitions: SupportTransition[],
manualTargetMode, reservedOriginalLeaseCapabilityId?,
observedWorkingInfobaseIdentity?: ManualWorkingInfobaseIdentity,
expectedOriginalFingerprint, observedOriginalFingerprint,
observedRootDeltaDigest, historyPartition: RepositoryHistoryPartition,
selectiveUpdatePlan: SelectiveRepositoryUpdatePlan,
concurrentRoutineChanges[], disjointExternalSupportChanges[],
rootLockObservationMode,
previewRootLockObservation?: SupportRootLockObservation, lockGuardDigest,
manualActorLockInventoryProof?: ManualActorLockInventoryProof,
manualWorkingInfobaseClosurePlan?: ManualWorkingInfobaseClosurePlan,
updateRequired,
updateDigest }`. A cross-mode actor/infobase/original, un-attributable version,
unexpected root/content/layer/off-support change, or remaining root lock cannot
be accepted. `previewRootLockObservation` is required only for
`readOnlySnapshot` and absent for `applyGuardOnly`; neither preview mode replaces
the mandatory apply guard. If exact version/working-infobase attribution or the
acquire/recheck/release guard is not capability-proven, the preview stops as
`supportPreflightInconclusive` rather than trusting a user assertion.
The support-prerequisite plan has scope `supportRoot` and selects exactly the
configuration root. Its root delta may include the one authorized version plus
proven disjoint external-support versions in repository order; those external
states are retained and force the bound relevant-advance phase.
The history partition begins exactly at the authorization's
`expectedBeforeHistoryCursor`, contains the arming receipt's byte-identical
all-unrelated prefix, and ends at the repository state used by the selective
plan. The authorized entry is the first root/support entry after `armingCursor`;
a later matching entry cannot excuse an earlier root/support version. The
arming ID/digest/cursor equal the immutable receipt, so preview/apply cannot
splice another manual window.

The observed working-IB identity is required exactly for
`separateWorkingInfobase` and absent for `reservedOriginal`; it must equal the
authorization's full display/digest record so a lost response does not reduce
the proof to an opaque hash.
The closure plan is likewise required with literal state `materialized` exactly
in separate mode: the authorized version is already observed before this
preview. It is absent in reserved mode.
The reserved-original lease capability is required exactly in reserved mode,
equals the authorization, and is absent in separate mode; preview proves the
capability exists but does not pretend to acquire the terminal lease.

`rootLockObservationMode` is exactly `readOnlySnapshot` or `applyGuardOnly` and
is selected by the validated capability row, not by caller preference.
`lockGuardDigest` binds that preview mode, the mandatory guard capability,
authorization/conditional actor-lock baseline, repository/original anchors, and
the preview observation when one is available. Apply must reproduce the same
bindings and returns the actual guard postcondition proof; a preview-mode change
requires a new preview. The actor-lock baseline/proof is present only for
`reservedOriginal`.

A distinct apply operation requires `dryRun: false` and
`approvedUpdateDigest`, rechecks
the status/binding/clean anchors, journals intent, and returns
`RepositoryUpdateData { beforeAnchor, afterAnchor, changedRelevantObjects[],
changedUnrelatedObjects[], appliedStructuralChanges[], originalFingerprint,
updateReceiptId, resultingPhase, supportPrerequisiteReceiptId?, supportRootLockProof?:
SupportRootLockProof, manualActorLockInventoryProof?:
ManualActorLockInventoryProof, reconciledHistoryPartition:
RepositoryHistoryPartition, selectiveUpdateProof:
SelectiveRepositoryUpdateProof,
postReleaseObservedHistoryCursor: RepositoryHistoryCursor,
postApplyHistoryPartition: RepositoryHistoryPartition,
reservedOriginalTerminalizationProof?:
ReservedOriginalTerminalizationProof,
manualWorkingInfobaseClosureProof?: ManualWorkingInfobaseClosureProof,
deferredRepositoryAdvance?: DeferredRepositoryAdvance,
deferredAdvanceConsumptionReceipt?:
DeferredRepositoryAdvanceConsumptionReceipt,
updateDigest }`.
Both partitions, the selective proof, and the post-release cursor are required
for every update mode. `deferredAdvanceConsumptionReceipt` is required exactly
for routine apply while a deferred handle is current, its
`advanceObservationDigest` equals the handle's `observationDigest`, and it is
persisted atomically with verified application and handle removal; preview
never produces it. Its `terminalReceiptId` equals the terminal receipt that
published the current handle, `routineUpdateReceiptId` equals the enclosing
`updateReceiptId`, `resolvedHistoryPartitionDigest` equals the approved and
reconciled routine partition digest, and `resultingPhase` equals the enclosing
result. `deferredRepositoryAdvance` is
absent for routine and is
present exactly when a terminal support scan encountered the immediate
disallowed successor or a coverage gap; then its `fromCursor` equals both
`postReleaseObservedHistoryCursor` and the post-apply partition endpoint. The
support receipt and root proof are required together exactly for
`supportPrerequisite` and absent for `routine`; the actor-inventory proof is
additionally required only for
`reservedOriginal` and absent otherwise. The working-IB closure plan/proof is
required exactly for `mode=supportPrerequisite` whose authorization uses
`separateWorkingInfobase`, must reproduce that preview plan while the root guard
is held, and is absent for routine and reserved-mode results. Completed
support reconciliation is impossible until the mandatory guard proof has proven
the root acquired/rechecked/released; `reservedOriginal` also
requires the reserved actor's complete lock set restored to its empty baseline
and `reservedOriginalTerminalizationProof`. That proof is required exactly for
reserved support reconciliation, absent for routine/separate mode, equals the
root proof's bound digest, and proves the exclusive configuration lease was held
from final original inspection through durable consumption. A known busy lease
produces the typed no-effect stop; unknown acquire/inspection/release is
support-prerequisite recovery.
For support reconciliation/cancellation the selective proof's root target and
release are the same acquisition recorded by `SupportRootLockProof`, not a
second lock window. Routine refresh instead proves its own complete root-first
target/parent/referrer lock set inside `SelectiveRepositoryUpdateProof`.
`postApplyHistoryPartition` covers the complete range from the selective
proof's before cursor through the post-release observation; the reconciled
partition ends at that before cursor, so there is no unclassified gap. The
reconciled partition is byte-identical to the approved preview partition and
has `throughInclusive == selectiveUpdateProof.observedBeforeCursor`; any
pre-effect extension requires a fresh preview. For `supportPrerequisite` it has
`fromExclusive == authorization.expectedBeforeHistoryCursor` and contains the
single expected authorized entry plus only the permitted routine/external
support evidence below. For `routine` it instead starts at the preview's
`expectedHistoryCursor`/`beforeAnchor`, contains the approved routine
classification range, and may reproduce the deferred first
`authorizedSupport`/`invalid`/`corrective` classification as a new external
relevant baseline; no authorization field is referenced. The
result never claims equality with a later global head: a task-relevant tail
forces the mode's bound safe relevant-advance phase
(`localVerified` for normal main preparation), an unrelated tail may keep the
otherwise planned phase, and `abandonmentReady` remains that terminal precursor
while recording either tail for its next routine refresh.
For `supportPrerequisite`, apart from the single expected `authorizedSupport`
entry consumed by reconciliation, every entry in both ranges must be
`unrelatedRoutine`, `relevantRoutine`, or proven `externalSupport`. Routine
uses its separately approved classification range and never consumes a support
authorization. The post-release scan stops
immediately before the first other support/invalid/corrective/unattributed
entry. Terminal consumption and its receipt remain durable; the disallowed
successor is stored only as `deferredRepositoryAdvance`, forces the bound
relevant-advance phase, and must be the first entry of the next routine update.
It never reopens or freezes the consumed authorization.
The support receipt records the authorized version/actor, exact semantic
transition, arming receipt, and every preserved external-support observation,
consumes the armed authorization,
updates the original when needed, and then invalidates every Dn-and-later
artifact/session/verification/plan,
returning to the authorization's exact `postReconcilePhase` only when neither
partition contains relevant/external support history, and otherwise to its
bound `relevantAdvancePhase`. Main preparation
therefore requires a fresh distribution and supported rebase; inverse cleanup
enters `abandonmentReady`. Normally that phase permits only status, routine
refresh, or abandoned archive; a current cleanup authorization narrowly permits
its exact arming/reconciliation/cancellation, and a frozen one permits only
status/recovery.
It is never an automatic support edit or rollback of the human repository
version. When the approved plan contains exact
add/delete operations, the adapter may derive the platform's repository-update
structural confirmation; callers cannot request or widen it. That path requires
capability evidence. The call refuses stale plans, unowned local changes,
active task locks outside recovery, and any automatic database restructuring.
Routine apply acquires the approved root-first existing target/parent/referrer
closure, then uses the exact object set with ordinary latest
`/ConfigurationRepositoryUpdateCfg -Objects` semantics and no `-v` claim. It
rechecks the planned per-target state under those locks and compensates with
`repositoryUpdatePlanStale` before update on any mismatch. After the effect it
records the exact matching per-target repository revisions/fingerprints;
unselected or later versions are classified but never reported as applied. An
unexpected structural prompt is not forced. A topology that cannot prove this read-update-read target map cannot
advertise routine update. Every acquired guard is released in reverse order;
  known foreign locks return the typed routine-update lock-conflict stop after
  compensation and unknown acquisition/update/
release effects enter recovery.

Support-prerequisite apply revalidates the history/root semantic digest. Its
first external effect is always a journaled root-guard acquisition by the
reserved account; under that lock it rechecks the full repository history,
the arming-prefix/first-root rule, version/support/content delta, support graph,
and original fingerprint. In `reservedOriginal` it then acquires the bound
exclusive configuration lease only after proving the Designer session closed,
rechecks the original while that lease is held, and retains it through durable
authorization consumption; separate mode performs the analogous bound
working-IB closure proof. It then
selectively updates only the configuration root to the approved planned state,
reproven as the latest root state under that guard, using
`/ConfigurationRepositoryUpdateCfg -Objects` without
`-force`, verifies the root/support fingerprint and target revision map,
atomically consumes the authorization, then releases/verifies the configuration
lease and repository guard. The
guard excludes only new root/support versions; a concurrent non-root commit is
recorded by the post-release partition and remains unapplied input to the next
routine refresh. A topology without proven root-selective update semantics
fails closed before the manual window is offered. A foreign root lock returns
`manualSupportLocksRemain` with the authorization still armed. An unknown
acquire/update/release result enters `recoveryRequired`; it never becomes a
retryable mismatch.

Cancellation preview returns `SupportPrerequisiteCancellationPreviewData {
mode: supportPrerequisiteCancellation, purpose, originPhase, cancelledPhase,
relevantAdvancePhase, supportActionId, supportActionDigest,
armingReceiptId?, expectedArmingReceiptDigest?, armingCursor?,
priorSupportGateId, reason, beforeAnchor, observedRepositoryVersions[],
historyPartition: RepositoryHistoryPartition,
selectiveUpdatePlan: SelectiveRepositoryUpdatePlan,
partitionedRoutineChanges[], relevantRoutineChanges[],
disjointExternalSupportChanges[], preArmExternalChanges:
SupportPrerequisiteVersionObservation[],
expectedOriginalFingerprint, observedOriginalFingerprint,
expectedSupportGraphDigest, observedSupportGraphDigest,
rootLockObservationMode, reservedOriginalLeaseCapabilityId?,
previewRootLockObservation?: SupportRootLockObservation, lockGuardDigest,
manualActorLockInventoryProof?: ManualActorLockInventoryProof,
manualWorkingInfobaseClosurePlan?:
ManualWorkingInfobaseClosurePlan,
cancellationDigest, updateRequired, plannedResultPhase }`. The three arming
fields are required together exactly for `armed` and absent for `awaitingArm`;
when present they equal the immutable receipt. It is eligible only
when bounded history semantically classifies every intervening version. From
`armed`, it must prove that none contains this authorization's transition and
classify every other support transition as proven disjoint external support.
From `awaitingArm`, every complete root/support entry is instead
`preArmExternal`, bound to the still-pending action/order violation and
preserved without requiring a fictitious independent-owner receipt. With no root/support
change, the selective target set is empty; with a preserved external support
change it is exactly the root. The original and support graph must otherwise
match the classified baseline, and preview observation
plus mandatory apply-guard capabilities are proven. Separate mode additionally
requires the exact inspection/exclusive-lease plan for the bound human
working IB, with literal state `materialized`; reserved mode omits it.
Unrelated/relevant routine, proven disjoint external-support, and (only while
awaiting) `preArmExternal` versions are allowed only as the exact previewed partition, preventing a concurrent normal
commit from trapping the awaiting/armed action forever. In every case
`preArmExternalChanges` is the exact ordered projection of those entries,
is empty for `armed`, and is non-empty iff an awaiting partition contains a
pre-arm root/support version. In every case
`historyPartition.fromExclusive == authorization.expectedBeforeHistoryCursor`;
an armed partition contains the byte-identical arming prefix, and no range may
start later. Apply requires
`approvedCancellationDigest`; it always journals and acquires the root guard
first, reproduces the approved partition through
`selectiveUpdateProof.observedBeforeCursor`, and repeats
history/support/original checks while holding it. Reserved mode also acquires
the authorization-bound exclusive original lease after a closed-session proof
and holds it through durable cancellation; separate mode uses its closure lease.
It selectively
updates only the root when required to preserve a classified external-support
change, atomically changes authorization state to `cancelled`
under the guard, then releases/verifies it. Completed data is
`SupportActionCancellationData { supportActionId, purpose, manualTargetMode,
armingReceiptId?, armingReceiptDigest?, priorSupportGateId,
supportRootLockProof: SupportRootLockProof, cancellationReceiptId,
manualActorLockInventoryProof?: ManualActorLockInventoryProof,
reservedOriginalTerminalizationProof?:
ReservedOriginalTerminalizationProof,
manualWorkingInfobaseClosureProof?:
ManualWorkingInfobaseClosureProof,
beforeAnchor, afterAnchor, changedRelevantObjects[], changedUnrelatedObjects[],
appliedStructuralChanges[], reconciledHistoryPartition:
RepositoryHistoryPartition, selectiveUpdateProof:
SelectiveRepositoryUpdateProof,
postReleaseObservedHistoryCursor: RepositoryHistoryCursor,
postApplyHistoryPartition: RepositoryHistoryPartition,
deferredRepositoryAdvance?: DeferredRepositoryAdvance, resultingPhase,
reason, cancellationDigest }`.
The result's arming fields are present together iff cancellation began from
`armed`; the reserved-original proof is required exactly in reserved mode and
the working-IB proof exactly in separate mode. `deferredRepositoryAdvance` is
present iff the post-release scan stopped at the immediate disallowed
successor or at a history-coverage gap, with `fromCursor` equal to both
post-release endpoints. The coverage-gap variant carries no invented version.
Cancellation never applies routine non-root or structural history merely to
close the authorization; those entries remain classified input to the next
routine refresh. `resultingPhase` is
the authorization's bound `relevantAdvancePhase` when either the classified
partition or observed post-apply tail is relevant, contains external support,
or contains `preArmExternal`,
and its bound
`cancelledPhase` otherwise; the original is proven equal only to the selected
target revisions, never unconditionally equal to the later global head.
Both cancellation ranges permit only `unrelatedRoutine`, `relevantRoutine`,
proven `externalSupport`, or awaiting-only `preArmExternal`. The post-release scan stops before any other known
immediate successor or at a history-coverage gap, durably completes
cancellation, records the matching known-successor or `coverageUnknown`
`deferredRepositoryAdvance`, and forces `relevantAdvancePhase`. The next
routine preview must reproduce the known successor or first prove coverage and
discover it without inventing a version; only its approved apply consumes the
handle from the terminal receipt cursor. The old
authorization is never reopened.
Main-preparation values are respectively
`localVerified` and `synchronized`; cleanup values never upgrade their bound
safe ancestor. The
inventory and reserved-original terminalization proofs follow the same
reserved-only presence rules as reconciliation; the working-IB closure proof is required only for
separate mode, its exclusive lease is acquired while the root guard is held,
and both remain held until a proven already-clean state plus cancellation are
durable. The reserved proof's capability equals the authorization and its
digest equals the root proof binding. A busy lease or dirty state releases every acquired guard without
cancellation and returns the typed external-cleanup stop. A matching
attributable version requires normal prerequisite reconciliation only when the
action is `armed`. While `awaitingArm`, a complete such version is preserved as `preArmExternal` by the
explicit cancellation path; incomplete evidence remains the cancellation
inconclusive stop. A retained lock or unclassified changed original/support
graph cannot be cancelled around. Unknown guard/lease/release effects enter
armed `supportPrerequisite` recovery or awaiting-action
`preArmSupportCancellation` recovery according to the interrupted state,
without retroactively arming the action.

No observed authorized version returns `manualSupportActionPending`. A valid
version with a current root lock or changed manual-actor lock inventory returns
`manualSupportLocksRemain`.
Unproven attribution returns `supportPreflightInconclusive`. For an armed
authorization, immutable history
violations—target-mode mismatch (including reserved account/original use in
`separateWorkingInfobase`), wrong actor scope, unauthorized non-support content
(including configuration-root business properties),
unexpected/layer/off-support transition, or an unowned original difference—
return `manualSupportPrerequisiteInvalid`, freeze the
authorization, and create a reconcile-only recovery plan over every observed
version, original fingerprint, and lock. Status retains the complete frozen
authorization, per-version observations, and typed corrective instruction. A later correction cannot erase that
history and therefore never uses ordinary prerequisite retry. After explicit
external corrective repository versions (root-only unless exact unauthorized
content restoration targets are required), `repository.recover` may publish
only the disposition-bound result: `restoreThenReauthorize` preserves all valid
routine and proven externally owned support versions, inverses only this
action's invalid graph/content, cancels the action, and
returns to `cancelledPhase`;
`preserveExternalAndReauthorize` preserves the proven external baseline without
inverse and returns to `relevantAdvancePhase`; `restoreThenAbandon` removes only
this action's tainted deltas/editability and returns
only to `abandonmentReady`, with successful integration permanently forbidden.
Every result requires the complete history archived, the reserved original
equal to the finalization plan's selectively applied target-revision map, and
no lock/effect unknown.

### `unica.repository.planLocks` — `contained`

Request fields: `taskId`, `operationId`, `comparisonId`, `mergeSessionId`,
`expectedResolvedSessionDigest`, `verificationId`, and
`expectedVerificationDigest`, `supportGateId`, and
`expectedSupportGateDigest`, `expectedSupportGateHistoryEvidenceDigest` for a
valid `mainSandbox` verification and the
current `ready` support gate. `data` is
`LockPlanData { planId, mergeSessionId, resolvedSessionDigest, supportGateId,
supportGateDigest, supportGateHistoryEvidence: SupportGateHistoryEvidence,
verificationId,
verificationDigest, integrationSetId,
integrationEntries: RepositoryIntegrationEntry[], integrationSetDigest,
lockEntries[], relevantAnchors,
compatibilityMode, referenceClosureDigest, settingsDigest,
prevalidationDiagnosticsDigest, planDigest }`.
`RepositoryIntegrationReason` is the closed declaration-order enum
`canonicalDelta`, `ownershipClosure`, `referenceClosure`, or
`addDeleteSemantics`. `RepositoryIntegrationEntry` is the named closed
`$defs.RepositoryIntegrationEntry` tagged `oneOf` of:

- `rootModify { target: { targetKind: configurationRoot }, objectDisplay:
  RepositoryTargetDisplay, action: modify, reasons:
  RepositoryIntegrationReason[], requiredLockTargets:
  RepositoryTargetIdentity[] }`;
- `objectAdd { target: { targetKind: developmentObject, objectId:
  MetadataObjectId }, objectDisplay: RepositoryTargetDisplay, action: add,
  reasons: RepositoryIntegrationReason[], requiredLockTargets:
  RepositoryTargetIdentity[] }`;
- `objectModify { target: { targetKind: developmentObject, objectId:
  MetadataObjectId }, objectDisplay: RepositoryTargetDisplay, action: modify,
  reasons: RepositoryIntegrationReason[], requiredLockTargets:
  RepositoryTargetIdentity[] }`; or
- `objectDelete { target: { targetKind: developmentObject, objectId:
  MetadataObjectId }, objectDisplay: RepositoryTargetDisplay, action: delete,
  reasons: RepositoryIntegrationReason[], requiredLockTargets:
  RepositoryTargetIdentity[] }`.

The nested `target` is exactly a `RepositoryTargetIdentity` leaf, not another
identity vocabulary. `objectDisplay` may reproduce the metadata name but is
presentation only and never selects identity, action, equality, or order. Every
reason list is non-empty, duplicate-free, declaration-ordered, and backed by its
matching canonical-delta/ownership/reference/add-delete producer evidence.
Every `requiredLockTargets` list is canonical and unique in repository-target
order and equals that entry's exact existing root/parent/referrer/target closure.
An added target does not yet exist to lock. A deleted development object's own
target is included if and only if capability evidence proves it still exists and
is separately lockable at acquisition; its parent, subordinate-development-
object, and every changed-referrer target remain mandatory. Absence of a proven
delete self-lock never removes the `objectDelete` integration/commit entry.
`integrationEntries` is non-empty, canonical, and unique by nested target in the
same repository-target order.

`CommitExactObject` is the named closed `$defs.CommitExactObject` tagged `oneOf`
of `rootModify { target: { targetKind: configurationRoot }, action: modify }`,
`objectAdd { target: { targetKind: developmentObject, objectId:
MetadataObjectId }, action: add }`, `objectModify { target: { targetKind:
developmentObject, objectId: MetadataObjectId }, action: modify }`, or
`objectDelete { target: { targetKind: developmentObject, objectId:
MetadataObjectId }, action: delete }`. It is exactly the nested target/action
projection of `RepositoryIntegrationEntry`; no display, reason, lock target, or
other planning field is representable. The projected list is non-empty,
canonical, and unique by target.

Every lock entry names a `RepositoryTargetIdentity` to acquire. The root guard
is legal, mandatory, and first; following entries name existing development
objects. Added and deleted objects remain in the integration set even when the
target itself is not separately lockable; the exact conditional delete self-
lock and mandatory surrounding closure above still apply. Broad
unexplained configuration locking fails. Every main-integration plan includes
the root as an explained lock entry with reason `supportGraphGuard`, even when
the observed support graph is empty and it is not a task-content integration
entry: the guard freezes both presence and absence of support. Its release
behavior must have exact capability evidence. `lockEntries` is the canonical
acquisition order and places that root guard first.

### `unica.repository.lock` — `journaledEffect`

Request fields: `taskId`, `operationId`, `planId`, and `approval:
DigestApproval` for `planDigest`. `data` is `LockResultData { planId,
planDigest, integrationSetId, integrationSetDigest, lockSetId, acquired[],
supportGateId, supportGateDigest, supportGateHistoryEvidence:
SupportGateHistoryEvidence, relevantAnchors, lockSetDigest }` only when the
complete set is owned. Acquisition rechecks the gate before the first effect.
The mandatory `supportGraphGuard` is always acquired first; while holding it,
Unica re-reads and matches the support graph/gate before attempting any
other object lock. A mismatch releases and verifies that root lock and returns
`supportPreflightStale`; unverified release enters `recoveryRequired`. Only then
are remaining locks acquired in deterministic order, so no object lock can be
based on a support graph that changed before the root was frozen.

A foreign lock returns the common `stopped` variant with
`RepositoryLockConflictData { failedTarget: RepositoryTargetIdentity, failedTargetDisplay: RepositoryTargetDisplay, lockedBy: RepositoryOwnerIdentity | null, diagnostic,
requestedExternalAction, acquiredThenReleased[], compensationVerified: true,
relevantAnchors }` and phase `blockedByForeignLock`. Failed compensation instead
returns `RepositoryLockRollbackFailedData { failedTarget: RepositoryTargetIdentity, failedTargetDisplay: RepositoryTargetDisplay, acquired[],
released[], retained[], retainedLockSetId?, retainedLockSetDigest?,
recovery: RecoveryPlanStatus }` and phase `recoveryRequired`; retained lock fields are required
together when non-empty. Partial/conflict data never uses the success variant.
Acquisition is per object with the bounded profile timeout and no polling; the
whole acquire/compensate operation also has the independently bounded
transaction deadline. No merge starts without exact final ownership proof.

### `unica.repository.unlock` — `journaledEffect`

Request fields: `taskId`, `operationId`, `lockSetId`, `expectedLockSetDigest`,
`reason` (`compensation`, `rollback`, or `abandonment`), and `approval:
DigestApproval` whose digest equals `expectedLockSetDigest` for the complete
currently owned set. Callers cannot select a smaller/broader subset. `data` is
`UnlockData { released[], retained[],
releaseVerified, originalRestored, unlockReceiptId }`. No force and no
unattributed/same-user pre-existing lock are allowed. From
`staleRelevantBaseline`, verified complete release plus proof that no original
merge occurred invalidates Dn-and-later evidence and returns `localVerified`.
From `staleSupportPreflight`, the same complete-release proof invalidates the
main session, verification, plan, lock, and gate evidence and returns
`synchronized` only when any changed original fingerprint has its bound
`OriginalCleanRefreshProof`; a fresh main preparation/preflight/plan is
required, but a fresh Dn is not unless the relevant partition later changes.
An unowned/local/unknown original delta instead enters `recoveryRequired` and
can never use this phase as a shortcut.
From `lockPlanExpansionRequired`, the same proof invalidates main-session,
main-verification, plan, and lock evidence and returns `synchronized`. Any
retained/ambiguous lock or original difference enters `recoveryRequired` with
an exact recovery plan. `reason="abandonment"` is valid from `locked` only when
the original still equals its pre-merge anchor; verified full release returns
`synchronized`. After an original merge, unlock alone is rejected and the
archive-previewed restore-plus-unlock plan is required.

### `unica.repository.commit` — `previewedJournaledEffect`

Request fields: `taskId`, `operationId`, `integrationSetId`,
`expectedIntegrationSetDigest`, `lockSetId`, `expectedLockSetDigest`,
`verificationId`, `expectedVerificationDigest`, `mergeReceiptId`,
`supportGateId`, `expectedSupportGateDigest`,
`expectedSupportGateHistoryEvidenceDigest`,
`expectedAuthorizedPostMergeFingerprint`, and `dryRun?: true` for
preview. Preview derives the immutable profile-rendered comment and returns
`CommitPreviewData { exactObjects: CommitExactObject[], guardLocks[], comment,
integrationSetDigest,
exactObjectsDigest, verificationDigest, lockSetDigest, mergeReceiptId, supportGateId,
consumedSupportGateDigest, supportGateHistoryEvidenceDigest,
authorizedPostMergeFingerprint, observedOriginalFingerprint,
historyGuardEvidence: PostMergeHistoryGuardEvidence, commitDigest }`.
`exactObjects` is byte-for-byte the non-empty canonical target/action projection
of the approved `LockPlanData.integrationEntries`; additions and deletions remain
present even when they have no separately acquired self-lock.
`guardLocks` contains acquired lock-only entries such as an unchanged
`supportGraphGuard` and is disjoint from task content unless the canonical
delta also changes that object. A
distinct apply operation requires `dryRun: false` and
`approvedCommitDigest`; applied
`data` is `CommitData { commitReceiptId, repositoryVersion,
beforeRepositoryCursor, afterRepositoryCursor,
postMergeHistoryGuardEvidenceDigest, postCommitHistoryPartition:
RepositoryHistoryPartition, atomicCommitSafetyCapabilityId,
committedObjects: CommittedRepositoryObject[], committedObjectsDigest,
contentVerified: true, releasedObjects[], releasedGuardLocks[], unlockVerified,
repositoryAnchor }`.
`CommittedRepositoryObject` is the named closed
`$defs.CommittedRepositoryObject` tagged `oneOf` of:

- `rootModify { targetKind: configurationRoot, action: modify,
  repositoryVersion: RepositoryVersion, targetFingerprint }`;
- `objectPresent { targetKind: developmentObject, objectId: MetadataObjectId,
  action: add | modify, repositoryVersion: RepositoryVersion,
  targetFingerprint }`; or
- `objectAbsent { targetKind: developmentObject, objectId: MetadataObjectId,
  action: delete, absenceEstablishedAtVersion: RepositoryVersion,
  expectedAbsent: true }`.

No leaf accepts a display, relevance/reason/lock field, a root `objectId`, an
absent-target fingerprint, or another leaf's version field. `committedObjects`
is non-empty, uses canonical repository-target order (the optional unique root
first, then lowercase canonical object UUID), and rejects duplicate target
identities. Its exact `(targetKind, objectId-if-applicable, action)` projection
produces the corresponding `CommitExactObject`. The required equality chain is
`projectIdentityAction(LockPlanData.integrationEntries) ==
CommitPreviewData.exactObjects ==
projectIdentityAction(CommitData.committedObjects)` byte-for-byte and one-to-
one; no target/action may be added, omitted, duplicated, reordered, or changed
between plan, preview, and result. Only the final projection source adds
authoritative version/fingerprint/absence post-state. Every present leaf's
`repositoryVersion` and every absent leaf's `absenceEstablishedAtVersion` equal
`CommitData.repositoryVersion`. Each `targetFingerprint` is the target's exact
post-commit fingerprint at that version; `objectAbsent` instead proves absence
at that same version, and `contentVerified: true` covers the complete post-state
projection. Independently, `exactObjectsDigest` equals the approved
`CommitPreviewData` identity/action-only digest and the equality chain above is
mandatory. It does not replace full integration-set lineage.
`CommittedObjectsDigestRecord` is the closed `{ integrationSetDigest,
committedObjects: CommittedRepositoryObject[] }`, where `integrationSetDigest`
equals the approved `LockPlanData`/`CommitPreviewData` value, and
`committedObjectsDigest == sha256(canonical(CommittedObjectsDigestRecord))`.
Thus the task-commit digest retains the established approved full integration-
set binding, while `exactObjectsDigest` separately proves that no presentation,
reason, or lock field entered the identity/action projection itself.
Supplying a comment is rejected; changing task metadata/template after start or
a render that is empty/not task-bound returns `commitCommentPolicyMismatch`.
Release, no force, no reference clearing, and no `keepLocked` are invariants,
not input switches. The adapter must prove that releasing an unchanged guard
does not add it as unrelated repository content; an unproven topology returns
`platformCapabilityUnproven` before lock acquisition.
`exactObjectsDigest == sha256(canonical(exactObjects))`; because every element is
a `CommitExactObject`, this semantic commit-object hash contains only canonical
target identity/action and cannot absorb display, reason, or lock data.
`commitDigest == sha256(canonical(preview-without-commitDigest))` separately
binds `integrationSetDigest` and the full comment/lock/fingerprint/history-
evidence approval inputs, so those cannot be substituted. The history
guard's `mergeReceiptCursor` equals both the original merge receipt's
`repositoryHistoryCursor` and the consumed gate evidence's
`classifiedThroughCursor`.

Both preview and apply immediately before commit intent remeasure the original
fingerprint, scan every repository version since the merge receipt, recompute
the task/reference/support closure, and require the
`consumedByOriginalMerge` gate to bind the supplied history evidence, merge
receipt, verification, integration/lock sets, and authorized post-merge
fingerprint. A proven unrelated tail is retained in
`PostMergeHistoryGuardEvidence`; a relevant/referrer tail returns
`postMergeLineageChanged`. Apply does not supply an expected repository cursor
to Designer: that public command has no such CAS. Instead it supplies the exact
objects under the already-held locks, without `-force`, to the
capability-proven atomic commit-safety boundary. The real fixture must prove
that a concurrent conflicting revision/referrer causes zero task objects to be
committed and that partial task versions are impossible; unrelated commits may
interleave. Drift observed before intent returns the evidence-bearing
`postMergeLineageChanged` stop and enters the exact original-restore plus
full-unlock plan. After the command, Unica scans the complete interval, locates
the task's exact immutable version/object set, and records unrelated
interleavings. A surprising relevant/conflicting interleaving or ambiguous own
version after an effect may have occurred is `committedUnverified` plus a
capability-breach observation plan, never a pre-effect stop, rollback, or blind
retry.
Because `commitDigest` covers the history evidence, an unrelated cursor advance
between preview and apply rejects the old approval and requires a fresh preview
without entering recovery; a relevant/referrer advance uses
`postMergeLineageChanged`. The immediate apply recheck then fixes
`beforeRepositoryCursor`; only races after that boundary are delegated to the
atomic-safety capability and post-commit partition.
For a completed apply, `beforeRepositoryCursor ==
historyGuardEvidence.classifiedThroughCursor`, and the approved history-evidence
digest/capability ID are reproduced exactly. `postCommitHistoryPartition` has
`fromExclusive == beforeRepositoryCursor` and `throughInclusive ==
afterRepositoryCursor`. It contains exactly one `taskCommit` entry whose
`repositoryVersion` equals the result field and whose `semanticDeltaDigest ==
committedObjectsDigest == sha256(canonical(CommittedObjectsDigestRecord))`;
that entry has no source-evidence ref and its committed-object identity/action,
version, and post-state equality is the exact enclosing `CommitData` validation
above. Every other entry is `unrelatedRoutine` or
capability-proven `nonConflictingConcurrent`. Any gap, duplicate/missing own
version, other relevant/support entry, object-set mismatch, or endpoint/digest
substitution is `committedUnverified`, never completed success.

### `unica.repository.recover` — variant policy

Request is a strict decision union. `apply` requires `decision: "apply"`,
`taskId`, `operationId`,
`expectedRecoveryDigest`, and `approval: DigestApproval` whose digest equals
`expectedRecoveryDigest` for the already-journaled or abandonment-previewed
recovery action. It uses `journaledEffect`; at effect intent it atomically enters
`recoveryRequired`, and only an observed terminal postcondition may publish
`plannedResultPhase`. `RecoveryData` is a closed tagged `oneOf` sharing
`{ target, effectClass, priorOperationId, terminalObservations:
RecoveryObservation[], actions: RecoveryAction[], actionOutcomes:
RecoveryActionOutcome[], resultingPhase, remainingUnknowns: RecoveryUnknown[],
approvedRecoveryDigest, recoveryReceiptDigest }`:

- `generalRecovery { target: taskConfiguration | repositoryLocks |
  originalConfiguration | repositoryCommit | manualWorkingInfobaseLease |
  artifact | archive | cleanup, effectClass, cleanupReceipt?: CleanupReceipt,
  ...common }`, where the exact
  target/effect/action combination follows the recovery matrix and no support-
  specific field exists; `cleanupReceipt` is required exactly for terminal
  `target=cleanup` and absent otherwise; or
- `supportRecovery { target: supportPrerequisite, effectClass: reconcileOnly,
  supportRecoveryDisposition: SupportRecoveryDisposition,
  manualTargetMode: ManualSupportTargetMode,
  successfulIntegrationForbidden?: true, supportRecoveryGuardProof:
  SupportRecoveryGuardProof, reservedOriginalTerminalizationProof?:
  ReservedOriginalTerminalizationProof,
  manualWorkingInfobaseClosureProof?: ManualWorkingInfobaseClosureProof,
  deferredRepositoryAdvance?: DeferredRepositoryAdvance,
  supportRecoveryReceiptId, supportVersionObservationDigest,
  supportRecoveryFinalizationPlanDigest, ...common }`.
- `preArmCancellationRecovery { target: preArmSupportCancellation,
  effectClass: reconcileOnly, supportActionId,
  expectedSupportActionDigest, approvedCancellationDigest,
  armingReceiptAbsent: true,
  manualTargetMode: ManualSupportTargetMode,
  effectObservation: PreArmCancellationEffectObservation,
  preArmCancellationFinalizationPlan:
  PreArmCancellationFinalizationPlan,
  preArmCancellationFinalizationPlanDigest,
  preArmCancellationReceiptPlanDigest,
  finalizationRecheckEvidence:
  PreArmCancellationFinalizationRecheckEvidence,
  preArmCancellationCompletedProgress:
  PreArmCancellationFinalizationAttemptProgress,
  finalizationAttemptAuditDigest,
  supportRootLockProof: SupportRootLockProof,
  reservedOriginalTerminalizationProof?:
  ReservedOriginalTerminalizationProof,
  manualWorkingInfobaseClosureProof?:
  ManualWorkingInfobaseClosureProof,
  selectiveUpdateProof: SelectiveRepositoryUpdateProof,
  supportCancellationReceiptId, supportCancellationReceiptDigest,
  preArmRecoveryReceiptId, preArmRecoveryReceiptDigest,
  reconciledHistoryPartition:
  RepositoryHistoryPartition,
  postReleaseObservedHistoryCursor: RepositoryHistoryCursor,
  postApplyHistoryPartition: RepositoryHistoryPartition,
  deferredRepositoryAdvance?: DeferredRepositoryAdvance, ...common }`.

`target`, `effectClass`, any disposition, mode, actions, and result phase reproduce
the approved plan byte-for-byte, so the cleared-plan terminal envelope remains
self-discriminating. In the support variant the closure proof is required
exactly for `separateWorkingInfobase` and absent in reserved mode; it reproduces
the current materialized closure plan. The reserved-original proof is required
instead exactly for reserved-mode terminal support recovery and absent in
separate mode, is byte-identical to the completed guard's nested proof, and its
capability/identity equal the frozen authorization. The forbidden-success
literal is required exactly for `restoreThenAbandon`. The guard variant is `completed`; its outcome is `cancelled` for
`restoreThenReauthorize` or `preserveExternalAndReauthorize`, and
`abandonmentFinalized` for
`restoreThenAbandon`. Its `finalizationPlanDigest`, the top-level
`supportRecoveryFinalizationPlanDigest`, and the approved recovery plan's
current finalization `planDigest` are identical; its nested selective proof is
identical to the materialized selective plan as specified above. The receipt and observation digest survive as status and
archive lineage after the recovery plan is cleared. Despite its historical repository
namespace, it is the single reconciliation entry for recorded repository,
authoritative original/task-IB, artifact/archive, or owned-cleanup effects. It
may reconcile or finish only a predetermined compensation, rollback,
restore/recreate, quarantine, or cleanup action; it cannot choose merge
resolutions, widen lock scope, force unlock, replay an unknown task mutation, or
retry an unknown commit.
In the pre-arm variant, the effect observation and full finalization plan are
byte-identical to the approved `finalize` plan and terminal status projection;
the compact plan/receipt-plan digests equal that plan's two nested digests. The
authorization terminates as
`cancelled` with no arming receipt, and the mode-specific proof presence follows
the same reserved/separate rule as ordinary cancellation. The root proof and
selective proof establish the exact recovered-or-completed effects and verified
release; neither may claim a support edit or repository version. The reconciled
partition equals `matched.observedHistoryPartition` for `matched`, equals
`releaseTailObserved.persistedHistoryPartition` for `releaseTailObserved`, and equals
`safeTailExtended.combinedHistoryPartition` for that protected-update-ready
outcome. The first two evidence partitions are already required byte-for-byte
equal to `finalizationPlan.finalizationHistoryPartition`; terminalization uses
the exact full evidence record rather than reconstructing or selecting a
partition from an independently matching cursor/digest. In every branch
`reconciledHistoryPartition.throughInclusive ==
selectiveUpdateProof.observedBeforeCursor ==
postApplyHistoryPartition.fromExclusive`; for `safeTailExtended` this is the
combined partition's appended endpoint, never the base endpoint. The original effect observation remains
immutable audit evidence and is not falsely rewritten after a replan. For
`releaseTailObserved`, its persisted partition equals the plan partition and its
allowed appended partition is the exact leading segment of
`postApplyHistoryPartition`; the latter starts at the reconciled endpoint and
ends at `postReleaseObservedHistoryCursor`. A later disallowed successor or
coverage gap is represented only by `deferredRepositoryAdvance`.
`preArmCancellationCompletedProgress` is exactly the `completed` branch, has the
plan's attempt ID, reproduces `finalizationRecheckEvidence`, and its
`attemptAuditDigest` equals `finalizationAttemptAuditDigest`. Its full realized
receipt list is the current attempt's exact ordered `source=finalizationPlan`
projection; prior-operation receipts remain embedded only in the immutable plan
and are not duplicated. The full plan retains every prior compensated-attempt
audit and receipt. The receipt-plan digest, final recheck evidence, and completed
attempt audit are byte-identical to the approved plan/current terminal progress.
Every
`source=priorOperation` ref resolves to its embedded immutable receipt; every
`source=finalizationPlan` ref resolves to exactly one ordered action outcome
whose `preArmCancellationEffect` receipt has the same ID/kind/intent; no
receipt can satisfy two refs or replace a prior-operation ref. The
resolved root acquisition/release receipt IDs equal
`supportRootLockProof.guardReceiptId`/`rootGuardReleaseReceiptId` exactly. In
reserved mode the resolved mode-acquire/release IDs equal the corresponding
`ReservedOriginalTerminalizationProof` fields; in separate mode they equal the
corresponding `ManualWorkingInfobaseClosureProof` fields. Capabilities,
identities, attempt IDs, and receipt digests must also match their plan/action
lineage, so no proof from another guard window is legal. The
authorization-cancellation ref resolves to
`supportCancellationReceiptId`/`supportCancellationReceiptDigest`; the final
`recoveryFinalization` ref resolves to the distinct
`preArmRecoveryReceiptId`/`preArmRecoveryReceiptDigest` pair. The common
`recoveryReceiptDigest` separately hashes the whole terminal result and cannot
be substituted for the action receipt without creating a hash cycle.

The terminal selective proof has an exact discriminator mapping. If the
approved target set is empty, both target lists/maps are the canonical empty
projection, `updatePerformed: false`, both structural flags are false, and no
update receipt exists. For a non-empty plan with receipt-plan disposition
`alreadyExact`, the target lists/maps equal the non-empty plan,
`updatePerformed: false`, both update-receipt fields are absent, and
`structuralConfirmationUsed: false`; immutable source
`updateState=alreadyExact` reproduces its guarded plan/target/fingerprint/
capability evidence, while the original evidence cursor remains in the effect
observation and the terminal `observedBeforeCursor` is the current reconciled
recheck endpoint defined above. No earlier
stage can synthesize this disposition from a later recheck: its non-empty plan
uses `perform` and the approved idempotent selective invocation/ref. If the
observation already contains `updateState: applied`, the full
proof repeats that effect's plan/targets/map digests, before/verified
fingerprints, capability, structural flags, and
`updateEffectReceiptId`/`updateEffectReceiptDigest` byte-for-byte, then adds the
exact current recheck endpoint and root/mode release evidence. Otherwise only
`selectiveUpdateDisposition=perform` supplies the single matching
finalization receipt and the same plan-bound fields; `updatePerformed: true`.
No branch may combine `notRequired` with a non-empty proof, `alreadyExact` with
an update receipt/action, or an applied effect with a second update receipt.
Any post-release relevant/unknown-coverage tail is represented by the optional
deferred advance under the ordinary cancellation rule and forces the bound
relevant-advance phase. Terminal status/archive retain the effect observation,
cancellation receipt, finalization digest, and recovery receipt, so response
loss cannot turn this path into an armed recovery. The post-release cursor and
partition follow the ordinary cancellation endpoint rules and are reproduced
unchanged by status/archive.
`approvedRecoveryDigest == request.expectedRecoveryDigest`; it identifies the
approved immutable plan. `actions[]` equals that plan's ordered action union
byte-for-byte. `actionOutcomes[]` is its one-to-one ordered terminal projection;
`terminalObservations[]` is canonical, contains only `matches`, and its digest
set equals the union referenced by those outcomes. `remainingUnknowns` is empty
in completed data; a difference, unknown, missing action outcome, or
postcondition mismatch keeps the plan current and uses its typed stopped or
effect-unknown recovery result. `recoveryReceiptDigest ==
sha256(canonical(result-without-recoveryReceiptDigest))` and covers the approved
digest plus every concrete terminal observation, action outcome/receipt, proof,
the full pre-arm finalization plan/prior-attempt audits/completed receipt
progress when present, optional deferred advance, and resulting phase. For terminal support recovery,
`deferredRepositoryAdvance` is present iff the completed guard contains the
  same observation; its `fromCursor` equals the terminal receipt endpoint and it gates
the next routine update.

`cancelPendingPlan` requires `taskId`, `operationId`,
`expectedRecoveryDigest`, and `decision: "cancel"`, with no approval object. It
uses `localJournaled` and is legal only for a current no-effect
`abandonmentRecoveryRequired` preview while phase and all source anchors still
equal that plan. It returns `RecoveryPlanCancellationData {
approvedRecoveryDigest, cancellationReceiptDigest, resultingPhase }`,
invalidates the plan atomically, performs no external effect, and leaves the
recorded `mainMerged` or `mainValidated` phase unchanged.
`approvedRecoveryDigest == expectedRecoveryDigest`; and
`cancellationReceiptDigest ==
sha256(canonical(result-without-cancellationReceiptDigest))`. A
stale, effect-started, or any other recovery plan is not cancellable.

## Exact Lock-Plan Mapping

| Canonical change | Required repository development objects |
| --- | --- |
| Modify top-level object or non-development child/property | Owning top-level development object |
| Modify existing form/template represented as development object | That form/template object |
| Add top-level metadata object | Configuration root; the new object does not exist to lock yet |
| Add subordinate development object | Parent development object owning the collection |
| Add attribute/tabular section/dimension/resource/non-development child | Owning development object |
| Delete development object | Parent, subordinate development objects, and every changed referrer are mandatory; include the deleted object's own target if and only if capability evidence proves it still exists and is separately lockable at acquisition. Lack of that self-lock never removes the `objectDelete` integration/commit entry |
| Delete non-development child | Owning object plus changed reference-cleanup closure |
| Change configuration-root property or top-level ordering | Configuration root |
| Preserve an accepted target support graph during integration | Configuration root as `supportGraphGuard`; this is a lock-only guard unless the canonical task delta also changes root content |
| Change serialized/reference property to a new object | Referencing development object |

Planner inputs are the platform comparison report, canonical UUID/property
delta, metadata ownership graph, reference graph, compatibility mode, generated
merge settings, current `ready` support-gate digest, and diagnostics from the
prevalidated main sandbox. Unknown ownership/reference kinds return
`unsupportedChangeKind`; file count, name-only mapping, or whole-configuration
fallback is forbidden.

`integrationSetDigest` covers every canonical add/modify/delete entry, its
reasons, the prepared main-session/result and support-gate digests, reference
closure, and required lock targets. It cannot cover the later authoritative merge receipt.
`lockSetDigest` covers only observed acquired locks. Original merge and commit
bind both digests, while the post-merge verification additionally binds the
authoritative merge receipt. Thus a newly added object cannot disappear merely
because it did not exist as an independently lockable repository object.

## Stable Error Contract

| Code | Trigger and required behavior |
| --- | --- |
| `repositoryBindingMismatch` | Original is not bound to expected repository; stop before mutation |
| `mainDiffersFromRepository` | Unowned local difference exists; stop before distribution/update |
| `artifactKindMismatch` | Verified artifact differs from explicit `expectedKind`, or is always-rejected `configurationUpdate`/`invalidArtifact`; retain classification evidence, publish no selectable verification handle, and do not advance |
| `artifactNotDistribution` | Dn is ordinary/configuration-update/invalid; do not deploy or synchronize |
| `platformWarningRejected` | A warning occurred in a delivery inspection/create/verification/deploy boundary; accept no delivery artifact/result. Configured merge-verification warnings instead become `invalid` and use `validationFailed` or `mainMergeValidationFailed` according to scope |
| `vendorAncestryMismatch` | The selected IDs already passed input/digest checks, but the authoritative task vendor no longer matches its D0/Dn checkpoint; enter `recoveryRequired` with an exact task-checkpoint restore/recreate/fingerprint plan whose successful result is `localVerified` |
| `twiceChangedProperties` | Explicit conflicts remain; enter `synchronizationConflicts`. When it coexists with unresolved references it is the primary `stopCode` and `errors[0].code`, while the same `MergeSessionData` retains every conflict |
| `unresolvedReferences` | Reviewed closure is incomplete; no implicit clearing. It is primary only when no twice-changed conflict remains |
| `unexpectedDelta` | Delta is missing/extra/unapproved; no lock acquisition |
| `adaptationDecisionAlreadyRecorded` | A different decision already binds the exact verification/difference; produce a fresh verification instead of overwriting audit history |
| `conflictDecisionsIncomplete` | Resolved replay lacks decisions for one or more exact conflict IDs; stay `synchronizationConflicts`, return all missing IDs, and perform no replay or sandbox recreation |
| `unboundResolutionChanges` | Resolved replay observed unconsumed or unbound changes in its exact resolution-workspace generation; remain `synchronizationConflicts`, bind valid changes through exact decisions or use the digest-bound `supportedUpdate` replacement subvariant to invalidate/recreate the whole conflict session; perform no replay |
| `validationFailed` | Local-checkpoint, synchronized-task, or pre-lock main-sandbox validation failed; create no checkpoint/verification success, retain immutable diagnostics, and remain in or enter `validationFailed` for repair in the task workspace |
| `repositoryLockConflict` | Foreign lock; compensate owned subset and request external release |
| `operationTimedOut` | Finite process deadline elapsed; after termination and observation, a proven-contained operation returns to its recorded safe phase, while an authoritative or unproven effect requires recovery; never auto-retry |
| `repositoryLockRollbackFailed` | Compensation is unverified; enter `recoveryRequired` |
| `repositoryUpdatePlanStale` | After the approved selective guard set is acquired, the exact per-target repository state/map differs from the approved plan; run no update, compensate every temporary guard, and require a fresh preview. Pre-guard status/binding/approval drift uses its ordinary stale-approval rejection instead of this domain stop |
| `repositoryStructureConfirmationUnproven` | Exact add/delete update requires an unproven platform confirmation path; perform no update |
| `manualSupportRequired` | The non-empty exact forward and/or task-surplus restore transition set is human-permitted with support retained; blockers may be empty in the surplus-only case. Publish only an `awaitingArm` authorization plus acquire-root/do-not-edit instruction, create no main session/lock, and remain `synchronized` |
| `manualSupportCleanupRequired` | Abandonment would retain task-only support transitions. Preview publishes only an inverse-only proposal/digest, with no authorization or external lease effect. The distinct approved apply runs journaled capability gates and may then publish the `awaitingArm` cleanup authorization plus acquire-root/do-not-edit instruction. While awaiting, permit only arming/cancellation/status; once armed, permit prerequisite reconciliation/cancellation/status. Remain at the origin phase and create no archive until reconciliation reaches `abandonmentReady` |
| `vendorForbidsChanges` | At least one exact candidate cannot change while retaining support, including an `offSupport`-requiring vendor deletion; create no main session/lock and require scope/vendor decision or abandonment |
| `supportPreflightInconclusive` | Required candidate/support-rule/diagnostic, recovery-distribution, actor, lease, or prerequisite-version evidence cannot be classified completely; this single semantic family fails closed without force or inferred outcome |
| `manualSupportRootLockRequired` | Arming observed no root lock or a proven wrong owner; publish no edit instruction/receipt, keep `awaitingArm`, and require the bound actor to acquire the root or coordinate the wrong owner's release |
| `supportPrerequisiteArmStale` | Before editing was armed, complete history/relevant baseline/support graph/recovery handoffs/original evidence changed. Preview and apply-final-recheck both leave `awaitingArm` unchanged, publish no edit instruction, and never cancel implicitly. Release any proven manual root lock, then run the explicit cancellation mode with its full terminal proof/receipt before fresh preflight |
| `manualSupportActionPending` | An armed action has no attributable version yet; perform no effect, keep it `armed`, and reconstruct the exact post-arm edit/commit instruction without polling |
| `manualSupportLocksRemain` | The manual-support window is not closed because the root remains locked or, in reserved mode, the complete reserved-actor lock set differs from its empty baseline; release and verify any task-owned guard, keep authorization awaiting/armed as applicable, and require exact external release before a fresh preview |
| `manualSupportLocalChangesRemain` | A separate working IB is capability-proven busy/open or dirty under its exclusive lease, or the reserved original cannot acquire its capability-proven closed-session lease; release every acquired guard, perform no reset/update/terminalization, keep the authorization/recovery current, and require human closure/cleanup. Unknown lease effects are recovery, not this stop |
| `manualSupportPrerequisiteInvalid` | Positively action-attributed immutable history violates the profile-bound target/content/support contract, or the explicit versionless singleton `originalNotClean` proves the reserved original differs under the terminal lease. Freeze and archive the whole available chain. Action-owned content/off-support taint forces restore-then-abandon, other action-owned invalid deltas and the versionless singleton restore-then-reauthorize, while proven external actor state is preserved; no ordinary retry erases an immutable version |
| `supportPrerequisiteConflict` | External support provenance/overlap is unattributed or conflicts with the authorized layer/target; freeze without reversal or terminalization and require digest-bound external correction/ownership evidence |
| `supportCorrectionPending` | Frozen support recovery still lacks its exact corrective version/destination; persist the recomputed finalization plan and wait without performing the human correction |
| `supportRecoveryReapprovalRequired` | Appended history, external-ownership reclassification, and/or working-IB closure materialization changed the frozen finalization/closure plan or digest; identify all applicable reasons, perform no finalization effect, and require explicit approval of the fresh recovery digest |
| `recoveryReapprovalRequired` | A conclusive observation selected a different predeclared recovery branch: commit observed committed/not-committed, pre-arm cancellation observation materialized its exact no-arming finalization, or a replannable pre-update finalization recheck changed after its newly acquired guards were receipt-proven released. Persist the new exact action plan/audit, perform no update/cancellation branch effect, and require explicit approval of its fresh digest |
| `supportConflictResolutionPending` | Frozen external-support conflict still lacks a proven corrective sequence or ownership receipt; preserve the complete chain and perform no automatic inverse |
| `supportRecoveryBlockedByLock` | The root-first recovery finalization set encountered a known foreign lock; compensate the acquired prefix, keep authorization unchanged, and require exact external release |
| `preArmCancellationRecoveryBlocked` | A no-arming cancellation finalization conclusively hit a foreign root guard or a busy/dirty mode lease before update; receipt-prove the empty/released prefix, append the attempt audit, persist the closed blocker/evidence/external instruction in the fresh current recovery plan for status replay, keep authorization frozen, and require external resolution plus approval of the fresh attempt. Unknown acquire/release effects remain recovery |
| `supportPrerequisiteReconciliationRequired` | An awaiting/armed external action may still require arming, may already have produced a version, or may require explicit cancellation; reject task mutation/abandonment/other authoritative work until its exact arm/reconciliation/cancellation path terminalizes it |
| `relevantBaselineChanged` | The relevant digest differs or the complete intervening partition contains any relevant/support/invalid/corrective entry, including change-then-revert with equal net digest; before locks invalidate synchronization evidence and return `localVerified`, or retain/release the full owned set from `staleRelevantBaseline`; fresh Dn is required |
| `supportPreflightStale` | A previously `ready` non-anchor gate input changed while the relevant digest and complete intervening history remain all-unrelated; return endpoint-bound history plus exact expected/observed inputs. A cursor advance alone is not stale. `originalFingerprintChanged` additionally requires a clean repository-refresh proof and no task merge; unowned/unknown original state enters recovery. Before locks invalidate main evidence; after locks retain the set in `staleSupportPreflight` until exact unlock |
| `mainPreparationMismatch` | Main sandbox has a conflict, unexpected scope, support-isolation violation, or extra repair; publish immutable difference evidence, enter `validationFailed`, and create no main session/lock plan |
| `additionalLocksRequired` | Before original mutation, enter `lockPlanExpansionRequired` with the exact retained lock set; verified full unlock returns `synchronized` and invalidates main preparation/plan evidence |
| `mainMergeValidationFailed` | Post-original-merge validation failed; enter `recoveryRequired` with an exact rollback-plus-unlock plan, and reach `validationFailed` only after `repository.recover` proves restoration and release |
| `postMergeLineageChanged` | Before commit preview/effect, the consumed gate/merge receipt or authorized post-merge fingerprint no longer matches; start no commit, enter `recoveryRequired`, and restore/unlock through the exact plan |
| `repositoryCommitFailed` | Enter `commitBlocked`; when capability evidence proves zero task commit, publish the exact original-restore/full-unlock branch, otherwise first use the observation branch. No retry/unlock shortcut |
| `repositoryCommitAmbiguous` | Enter `committedUnverified` with the exact observe-only plan; a conclusive observation publishes a separately approved committed-release or not-committed restore/full-release branch. No retry/cleanup |
| `repositoryUnlockUnverified` | From commit enter `committedUnverified`; from standalone unlock enter `recoveryRequired`. Both publish an exact observation/compensation plan and allow no archive/cleanup until reconciled |
| `cleanupNotAllowed` | Terminal/archive proof is incomplete; retain data |
| `abandonmentRecoveryRequired` | Abandonment was requested after original merge; preview the exact rollback-checkpoint plus full-unlock plan, leave phase unchanged, and require approved `repository.recover` before archiving |
| `unsafeTaskPath` | Marker/path/root/reparse guard failed; perform no destructive action |
| `operationReplayMismatch` | Same operation ID has different canonical input; reject |
| `targetReservationBusy` | Authoritative target reservation is held by the closed redacted owner reference; create no task and return its exact reservation context with `statusOnly` |
| `repositoryAccountReservationBusy` | Authoritative repository-account reservation is held by the closed redacted owner reference; create no task and return its exact reservation context with `statusOnly` |
| `operationEffectUnknown` | External effect is unresolved; only recovery may proceed |
| `recoveryPlanPending` | One exact recovery plan is current; reject every mutation except its `repository.recover` apply, or the cancellation variant when it is a no-effect abandonment preview |
| `taskPhaseMismatch` | A lifecycle/merge/repository precondition, or a clean bridge phase with no safety blocker, does not allow the tool; perform no effect and return exact allowed phases |
| `approvalDigestMismatch` | A syntactically present preview/session/plan/lock/recovery approval carries a stale or different digest; perform no effect and request a fresh producer call. Omission of a schema-required approval is an MCP request-schema error before task lookup/result and cannot use this code |
| `changeReceiptStale` | A merge-resolution receipt is no-change, consumed, superseded, from a stale workspace/session generation, or mismatches the exact target/result hash; or ordinary descendant evidence invalidated a required receipt. Perform no decision/effect and recreate or review the exact producer result |
| `conflictResolutionNotAllowed` | The requested closed resolution is not a member of the exact conflict's persisted non-empty `allowedResolutions`; reject before receipt inspection/consumption or decision mutation and require a fresh conflict-list review |
| `taskMutationBlocked` | Exclusively for a compatible general mutation with no current recovery: worker, original difference, owned lock, or terminal phase is a safety blocker; this code wins over phase mismatch and requires status plus cleanup/closure or a new task. A current unknown effect always uses higher-precedence `recoveryPlanPending` |
| `platformCapabilityUnproven` | Exact topology/platform row missing or stale; disable mutation |
| `supportLayerAmbiguous` | Exact vendor layer cannot be selected/round-tripped; no edit |
| `unsupportedChangeKind` | Ownership/delta/reference behavior is unimplemented; no approximation |
| `projectIdentityCollision` | Project ID maps to different canonical targets; refuse shared state |
| `stateRootRelocationRequired` | Project locator names a different durable root; block until explicit offline migration |
| `exclusiveRepositoryUserRequired` | Profile/account exclusivity is absent or contradicted; stop preflight |
| `rollbackUnproven` | Required restore/unlock postcondition cannot be proven; recovery required |
| `taskAbandonmentNotSafe` | Worker/lock/difference/unknown effect remains; no abandoned archive |
| `profileInvalid` | Local schema, topology, path, or inline-secret rule failed; create no task |
| `secretUnavailable` | A referenced secret is absent/empty; create no process or durable secret derivative |
| `stateCorrupt` | Durable schema/hash/permissions are invalid, including any stored operation record whose policy is `readOnly`, whose physical `operation` selector is read-only, or whose operation/policy/terminal-envelope binding disagrees; reject before storage projection/replay dispatch and perform no external effect |
| `operationInProgress` | A live recorded operation/lease owns the target; attach/status instead of spawning |
| `taskNotFound` | A non-start lifecycle tool references `notCreated`; advertise the exact canonical safe-action set `[branched.start, branched.status]`, whose order is not an execution sequence |
| `taskWorkspaceContextInvalid` | `branchedTask` ID, marker, project binding, or lease authenticity does not match; do not expose or use a path |
| `toolNotBranchedCompatible` | A general tool has not declared `supportsBranchedTask`; reject before dispatch |
| `commitCommentPolicyMismatch` | Frozen template/task metadata cannot produce the exact task-bound comment; do not commit or refresh a producer. Return status plus the safe abandoned-archive preview, which must publish the exact restore/full-unlock plan before recovery apply |
| `integrationSetMismatch` | Plan, merge, verification, commit set, or lock set lineage digests disagree; do not refresh a producer or retry commit. Return status plus exact rollback unlock when the complete set is held and no original merge began, otherwise enter the exact recovery plan |

Raw localized text is supporting redacted evidence, never the stable code.
Error selection is deterministic: context identity/authenticity failure returns
`taskWorkspaceContextInvalid`; after authentic task lookup a current recovery
plan returns `recoveryPlanPending` for every mutation except its exact recover
apply or permitted no-effect cancellation. Otherwise an authentic compatible
mutation with a safety blocker returns `taskMutationBlocked`; only then can an
otherwise clean wrong-phase call return `taskPhaseMismatch`.
