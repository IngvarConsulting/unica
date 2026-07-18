# Task 8 v5 — fresh independent adversarial design review

Date: 2026-07-18

## Integrity and review boundary

- Reviewed artifact: `.superpowers/sdd/task-8-design.md`.
- Required and observed SHA-256, checked before the review:
  `ed31f6d9714a6be8890202e4e8181560e195bdcdeee85277daabb2052537f3e3`.
- The whole 4,415-line artifact was read. The older Task 8 reviews and the
  author's closure were not used as acceptance authority.
- At review start, frozen prerequisite anchors were checked independently:
  - Task 5B v5 contract:
    `13ca8e3599ce3e4843ae82773a8911194f2786ce741b9040c14563b60dbedbab`;
  - Task 7 v5 design:
    `6792d70c58a57a35871a91f5dd9059371ee13599a96e0c00e97e27a974f6ca2a`;
  - Task 5B v5 self-audit:
    `d9d866094e4d5587751dd853688ec85db3c14c6db40564f9f372bedebcc23f30`.
- This reviewer edited no tracked file or design artifact. This ignored review
  note is the only file created by this review.

During final verification, an authorized shared-worktree design-fix agent
concurrently replaced `.superpowers/sdd/task-5b-contract.md` in place with a v6
candidate. The coordinator confirmed that the in-place target was a coordination
mistake, stopped the writer, and will restore/archive v5 separately before
continuing v6 only in new versioned paths. The later content was not used as
review authority and was not modified or restored here. This incident is not
counted as a Task 8 semantic finding. Task 8 v5 itself remained byte-exact
throughout.

Platform claims were checked far enough to avoid reporting API-shape false
positives. The local macOS SDK defines `_PC_CASE_SENSITIVE`,
`renameatx_np(RENAME_EXCL)` and `F_FULLFSYNC`; directory `fsync` also succeeded
on the local APFS runtime. The pinned `windows-sys 0.52.0` sources contain
`NtFlushBuffersFileEx`, `NtQueryVolumeInformationFile`, `FileRenameInfoEx` and
`FILE_RENAME_FLAG_REPLACE_IF_EXISTS`. Current Microsoft `FILE_RENAME_INFO`
documentation also permits a directory handle in `RootDirectory` for a relative
new name. Those primitive choices are therefore not findings. The design's
native per-tuple race/failure/crash qualification remains mandatory.

## Verdict

**NEEDS-FIX — Task 8 production implementation remains a hard STOP.**

V5 closes the reviewed v4 defects around staging-name consumption, independent
durability, root-wide alias serialization, internal mount rejection, Windows
handle-relative mutation and the Task 9/10 lock order. It still has two P0
state-machine/crash holes and one P1 effect-contract hole. In particular, a
process crash can leave an invisible staging file with no recovery authority,
and a definite commit followed by an unqueryable post-install observation has
no honest value in the supposedly total outcome algebra.

The already-declared missing accepted Task 5A SHA remains an independent
prerequisite STOP (`task-8-design.md:4193-4199`). It is not counted again as a
new v5 design finding.

## P0 findings

### P0-1 — a process crash loses staging ownership and permits a later false-clean advance

Evidence:

- The writer creates an unpredictable temp name and records its
  `OwnedStagingIdentity` only as part of the live call before writing
  (`task-8-design.md:3162-3167`).
- Typed authority is returned through `HandlerOutcome`; returning no mutation
  after the first filesystem mutation is forbidden
  (`task-8-design.md:2946-2953`). A hard process/guest crash returns no outcome
  at all, so these constructors cannot preserve the identity.
- Task 8 explicitly implements no receipt persistence or observation-journal
  storage (`task-8-design.md:1007-1016`). Task 9 persists the completed generic
  outcome schema, while Task 10 transitions a receipt from the returned outcome
  (`task-8-design.md:826-855`); neither section defines a durable pre-mutation
  intent/staging record or restart recovery.
- The design explicitly says that the ordinary catalog may ignore random temp
  names and that Task 10 learns residue only from the exact typed path returned
  by the live writer (`task-8-design.md:840-843`).
- Forced-crash suites are required as backend evidence
  (`task-8-design.md:3241-3248,4002-4005`), but no required post-restart action
  discovers, owns, removes, quarantines or blocks on a temp left by an
  interrupted call.

Counterexample:

1. All target parents already exist and the target still has baseline `S0`.
2. An applied call acquires the artifact lease, creates/writes temp `T`, then is
   killed before install, cleanup and `HandlerOutcome` construction.
3. OS locks are released. `T` remains in the source parent, but the target and
   all watched parent/target entries still equal `S0`; the random temp is not in
   the ordinary manifest.
4. A later call therefore matches the old receipt baseline, creates a different
   temp, commits the expected target, proves only its own staging lifecycle
   clean, and can satisfy the current Task 10 advance predicate while `T`
   remains.

The result is a source-tree effect that belongs to a previous Unica mutation but
is absent from both the post-manifest and the advancing outcome. Receipt-free
`off/observe/warn` calls have the same crash window, so a receipt-only in-flight
flag cannot close the artifact invariant.

Required correction:

- Add an artifact-writer crash protocol that is durable before the first
  staging name can be created. A minimal solution is a versioned, bounded
  mutation-intent/recovery record in the fixed control universe containing the
  chosen staging name, retained-parent/target identity authority and phase. It
  is not the non-authoritative Task 11 observation journal.
- On every root-wide artifact lease acquisition, recover or fail closed on every
  unresolved intent before authoritative capture or new source mutation. A
  record cannot be cleared until the target/staging/parent state and required
  namespace durability are proven.
- Task 10 should additionally persist an enforceable receipt's in-flight/revoke
  transition before the handler, but this is supplementary; Task 8 still owns
  crash-safe staging recovery for receipt-free modes.
- Add hard-kill/restart REDs after temp create, partial write, data flush,
  install, staging removal and rollback removal. With pre-existing parents and
  an unchanged target, the restarted process must not resolve/advance another
  call while the old intent or staging name is unresolved.

### P0-2 — definite commit plus unknown post-install observation is uninhabitable

Evidence:

- `CreatedFile` and `UpdatedFile` require a workspace path, physical identity
  and known content digest; only the detached variant has a `Known | Unknown`
  content state (`task-8-design.md:2827-2869`).
- `Committed::new` requires exactly one definite target effect: a path-resolved
  Created/Updated row or a proven detached/relocated row. A definite commit may
  not be downgraded to `Uncertain` (`task-8-design.md:2965-2974`).
- The detached row is selected only when a fresh walk proves that the intended
  path is absent or resolves to another object
  (`task-8-design.md:3354-3369`). There is no `location=Unknown` case.
- The algorithm nevertheless requires target reopen/read-back, exact final
  digest/identity verification and a fresh root-relative path walk after the
  install (`task-8-design.md:3169-3174,3354-3360`). A definite rename can be
  followed by an I/O/access/observation failure at any of those steps.
- The mandatory failure matrix explicitly injects failure at `read-back` and
  requires an exact NoChange/Committed/Uncertain outcome with complete effects
  (`task-8-design.md:3890-3897,3997-4001`).

After a rename reports definite success, a failed target read or failed fresh
walk does not make install completion uncertain. But the writer also cannot
truthfully invent the planned content digest, claim the path is verified, or
claim the object is detached. None of the current variants is legal. Mapping
this state to `Err`, `Uncertain`, expected Created/Updated, or detached would
violate a different normative invariant and could either lose a definite effect
or falsely authorize receipt advance.

Required correction:

- Model definite target authority independently from the observation result.
  For example, give the definite target effect closed
  `location = AtIntendedPath | DetachedOrRelocated | Unknown` and
  `content = Known(digest) | Unknown` states while retaining physical object and
  parent identity. Created/Updated path effects are constructible only for the
  fully verified `AtIntendedPath + Known` case.
- `Committed` must accept exactly one definite target object even when location
  or content is unknown. Any unknown/mismatch row is unexpected,
  non-advancing/revoking; only exact expected path/content plus
  `VerifiedClean + VerifiedDurable` can advance.
- Split failure REDs at target reopen, target read, identity query, fresh parent
  walk and path/object comparison after definite install. Separately retain
  `Uncertain` only for an install primitive whose completion itself is unknown.

## P1 findings

### P1-1 — Present replacement has no exact metadata-effect contract

Evidence:

- The final Present precondition checks bytes, length/digest/boundary and object
  identity, but no file metadata (`task-8-design.md:3123-3139`).
- The write sequence says only “preserve safe existing target permissions”
  without defining the captured fields, supported subset, rejection policy,
  digest or platform operation (`task-8-design.md:3162-3170`).
- `UpdatedFile` records object identity and before/after content digests only;
  the effect algebra has no mode/owner/ACL/xattr/basic-attribute/stream state
  (`task-8-design.md:2827-2855`).
- The Present backend tables and mandatory failure matrix exercise data,
  install, cleanup and durability but contain no metadata precondition/copy/
  verification cases (`task-8-design.md:3228-3236,3848-3897`).

Same-directory replacement installs the temp object, not the old target object.
Without an exact policy it can silently replace a Unix mode/ACL/xattr/file-flag
set or a Windows security descriptor/basic attributes/stream set with the
temp's inherited metadata. The post-manifest can still match the planned BSL
bytes and Task 10 can advance, even though the operation made an unmodelled
access or metadata change. “Safe permissions” is not an implementable or
reviewable invariant.

Required correction:

- Choose and specify one closed `PresentTargetMetadataV1` policy per enabled
  backend: either capture/copy/precondition/post-verify the supported metadata,
  or reject every target outside a narrowly defined ordinary metadata subset
  before staging creation.
- Bind that tuple/policy version into execution authority and report any
  authorized metadata delta in typed effects. Do not infer it from content or
  physical identity.
- Add Unix custom mode/ownership/ACL/xattr/file-flag and Windows custom security
  descriptor/basic-attribute/alternate-stream REDs, including a metadata race
  between the final check and replace. A receipt cannot advance when metadata is
  unknown or changed outside the explicit policy.

## No additional open finding in the requested boundaries

Subject to the findings above, the following v5 contracts are internally
consistent at design level:

- one fixed control root and root-wide physical-destination collision lock;
- case/NFC/NFD semantic-key separation and actual inode/FileId plus Busy proof;
- rejection of an internal destination mount/bind while allowing only proven
  whole-workspace aliases;
- no-follow/handle-relative parent and target containment;
- Absent atomic no-replace versus cooperative Present replacement;
- Removed versus ConsumedByTarget staging-name lifecycle after a returned call;
- independent `VerifiedClean` and target durability, with strict
  `RolledBackDurably` for NoChange;
- artifact-before-receipt acquisition and reverse release;
- raw assertion erasure from the final plan and handler prohibition on
  reparsing/rerendering;
- Task 8 pure seams versus Task 9 persistence and Task 10 guard policy, apart
  from the newly required artifact crash-recovery authority.

These conclusions do not qualify any platform row. Each row remains disabled
until its exact native two-process, race, failure and forced-crash/restart suite
and reviewed evidence digest exist.
