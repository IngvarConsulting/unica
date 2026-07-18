# Task 8 v4 — fresh independent adversarial design review

Date: 2026-07-18

## Integrity and review boundary

- Required source artifact:
  `.superpowers/sdd/task-8-design.md`.
- Required and observed source SHA-256:
  `5b2436cbb38af24a011410769a83e9dd00fdd60abd596cef82c06b6833639a01`.
  The hash was checked before review and checked again after the read-through.
- Accepted checkout anchor: `HEAD=20f6afa7a09430614babebc0cdeebeb94c8a0189`.
- Task 5B review anchor:
  `.superpowers/sdd/task-5b-v3-fresh-review.md` at
  `cd52d99e2b46c3328443148ec5d7b9be01b92bf22c8216eb37e732379f45e76a`.
- The uncommitted Task 5A diff was used only as unstable context. No conclusion
  below treats it as an accepted source of truth.
- `AGENTS.md` precedence was applied: code/tests/package metadata, then active
  spec, then historical plans.
- No tracked file was edited and no implementation was attempted.

The review checked every positive claim in the identity/lease, contained writer,
effect algebra, Form completeness, prerequisite and Task 9/10 ownership paths.
The Windows API shape was also compared with the current Microsoft contracts for
[`NtCreateFile`](https://learn.microsoft.com/en-us/windows/win32/api/winternl/nf-winternl-ntcreatefile),
[`SetFileInformationByHandle`](https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-setfileinformationbyhandle),
and
[`FILE_RENAME_INFO`](https://learn.microsoft.com/en-us/windows/win32/api/winbase/ns-winbase-file_rename_info).
Those references support relative opens through `OBJECT_ATTRIBUTES.RootDirectory`
and a relative rename target through `FILE_RENAME_INFO.RootDirectory`; therefore
the mere choice of these APIs is not reported as a defect. The native allowlist
and failure suite in the design are still mandatory.

## Verdict

**NEEDS-FIX — implementation remains a hard STOP.**

V4 closes many v3 holes: it puts artifact acquisition before current receipt
acquisition, keeps the destination handle alive, distinguishes definite target
commit from cleanup, refuses path-based Windows destination reopen, requires
Base+Own/Extension+Adopted authority, and keeps Task 9/10 persistence and guard
policy out of Task 8. However, two P0 contradictions make the advertised clean
receipt transition unsound or unconstructible, and three P1 contract holes leave
the shared Form authority and artifact serialization boundary unsafe. The design
must be revised and reviewed again before Task 8.0 or any production RED.

The independent prerequisite STOP also remains open: there is no accepted clean
Task 5A SHA, and the anchored Task 5B review is `NEEDS-FIX`. V4 correctly names
those as prerequisites, so this is an operational gate rather than an additional
Task 8 wording finding.

## P0 findings

### P0-1 — `VerifiedClean` requires the installed staging object to cease to exist

Evidence:

- A successful outcome may advance only as exact expected
  `Committed + VerifiedClean` (`task-8-design.md:791-800,3294-3299,3601-3607`).
- `VerifiedClean` records every staging identity, and the normative rule says it
  is issued only after both the staging name **and physical object identity** are
  proven absent (`2679-2692,2723-2733`).
- The Absent algorithm installs with `linkat(temp, target)` and then unlinks the
  temp name (`2858-2865`). The target is the very same inode/file identity as the
  staging object.
- Present replacement renames the already-open temp into the target name
  (`2839-2853,2885-2895,2924-2933`). The temp object identity likewise survives
  as the installed target object.

Impact:

After either successful install, the staging *directory entry* can be absent,
but the staging *object identity* must still exist as the target. Under the
literal constructor invariant, a normal successful commit cannot construct
`VerifiedClean`; under a looser implementation, the constructor would violate
the reviewed contract. This is not a diagnostic nuance: Task 10's only advancing
state becomes either impossible or dependent on an unchecked reinterpretation.

Minimal REDs:

1. Absent: create temp identity `T`, hard-link `T` as target, unlink the temp
   name, and prove that clean commit is constructible while target identity is
   still exactly `T`.
2. Present: atomically rename temp identity `T` over the old target, prove the
   temp name absent and target identity `T`, and construct the same clean commit.
3. Leave an additional temp-name entry for `T`; the result must be
   `Committed + Residue`, never clean.
4. Relocate the installed target while the temp name is absent; cleanup may be
   clean, while the target effect remains detached/non-advancing. Cleanup and
   target location must stay independent.

Required correction and ownership:

- Task 8 `mutation_effects` must model a staging **name lifecycle**, not demand
  global extinction of an object that was consumed by installation. A closed
  shape such as `Removed { staging_name, identity }` versus
  `ConsumedByTarget { staging_name, identity, target_effect }` is sufficient.
- `VerifiedClean` must require every owned staging name to be absent and every
  staging object identity to be accounted exactly once: either removed or equal
  to the definite target effect. Residue/Unknown continues to describe an extra
  staging name/object, not the installed target itself.
- Back-propagate the corrected invariant into the active spec/ADR, Task 9 schema
  boundary and Task 10 advance predicate/failure matrix.

### P0-2 — target durability is absent from the outcome and advance algebra

Evidence:

- The writer flushes temp data, commits, then performs a parent-metadata
  durability step (`2839-2850`).
- Failure injection explicitly includes `flush`, `rename/link/unlink` and
  `parent-sync` (`3358-3363,3450-3454`).
- `MutationHandlerOutcome::Committed` contains definite effects and only
  `MutationCleanupState`; it has no durability state (`2620-2692`).
- The advance predicate is true for expected `Committed + VerifiedClean` with a
  matching post-manifest and does not consult `AdapterOutcome.ok`
  (`3294-3299,3601-3607`).
- The prose classifies target certainty and staging cleanup, but never assigns an
  authoritative state to “rename/link succeeded, target reads back, staging is
  clean, parent durability sync failed” (`2908-2947,2949-2975`).

Impact:

That injected state has a definite current target effect, exact expected paths,
clean staging and a matching live post-snapshot. The current type can therefore
look advance-eligible even though the directory-entry commit is not proven
durable. A receipt record can be durably advanced and survive a crash while the
source-tree rename/link does not, breaking the baseline/receipt invariant.
Downgrading it to `Uncertain` would also contradict v4's correct rule that a
definite target commit must not be hidden merely because a later step failed.

Minimal REDs:

1. Inject parent-directory sync failure after a successful Present rename,
   successful target read-back and successful staging cleanup. The result must
   remain a definite commit but `receipt_advance_eligible` must be false.
2. Repeat for Absent link+unlink and for the final directory sync after unlink.
3. Prove data/temp flush failure before install remains effect-free NoChange.
4. For each claimed Windows/Unix filesystem tuple, prove the exact durability
   primitive. An unavailable or failed primitive must produce a typed
   non-advancing state before any receipt transition.

Required correction and ownership:

- Add a closed `MutationDurabilityState` (for example
  `VerifiedDurable | Unknown`) to definite `Committed` authority; do not overload
  staging cleanup or adapter presentation.
- Exact expected `Committed` advances only with `VerifiedDurable` **and**
  `VerifiedClean`. A parent-sync failure is definite `Committed`, durability
  Unknown, and non-advancing/revoking.
- Task 8 owns the type and platform writer classification. Task 9 persists only
  the agreed generic schema/version. Task 10 owns the production advance/revoke
  rule. Active spec, ADR, observability and replay text must carry the new field.

## P1 findings

### P1-1 — v4 still embeds the rejected second Form event registry

Evidence:

- V4 normatively defines its own `FormItemKindV1` and event-capability table
  (`409-426,482-509`). It marks `Button` event-capable and omits already-audited
  kinds such as `RadioButtonField`, `TrackBarField`, `ExtendedTooltip`, document,
  graphical-schema, HTML and spreadsheet fields.
- The accepted live registry at HEAD contains those kinds and marks
  `Page | Button | CommandBar | Group` as `NO_EVENTS`
  (`crates/unica-coder/src/infrastructure/native_operations/form_event_registry.rs:261-389`).
- V4 accepts an identifier-shaped `event_name`, treats missing callType as Direct
  and retains Actions by ordinal, but it never imports the live exact
  event-per-target matrix, regular-versus-extension `BaseForm` callType rule, or
  a proven Command Action cardinality (`374-401,490-529,510-522`).
- The anchored Task 5B review already rejects exactly this duplicate authority
  and zero-Action/generic-event behavior
  (`task-5b-v3-fresh-review.md:91-110`).
- V4 simultaneously says there must be one shared parser/registry
  (`523-531,3508-3509`), so the design contradicts itself.

Impact:

A worker following the explicit v4 enum/table can declare semantically invalid
Form XML complete, or reject valid already-supported material as unknown. That
invalidates both negative proofs required by Task 8: source Ordinary and
destination generated-name-unbound. The later sentence “reuse the shared
catalog” does not neutralize an earlier normative duplicate table.

Minimal REDs:

1. `Button/Click` is not accepted by the current matrix; every audited
   event-bearing kind, including `RadioButtonField/OnChange`, is consumed once.
2. Unknown event token for any known kind makes the complete view unavailable.
3. Regular Form plus explicit `callType` is incomplete; the same pair is accepted
   only under the exact proven extension/BaseForm semantics.
4. A Command with zero or duplicate Action is incomplete unless an authoritative
   fixture proves a narrower valid shape.
5. Task 8 contains no second item/event/callType table and consumes the exact
   registry contract/version produced by Task 5B.

Required correction and ownership:

Task 5B must first extract one neutral versioned Form definition/item/event/
callType/Action registry from the audited implementation and close its own P1.
Form edit/validate and the complete catalog both reuse it. Task 8 then deletes
the duplicated `FormItemKindV1` capability table from this design and consumes
only the complete typed projection plus registry version. Back-propagate the
single-registry requirement to Task 6 identity use, Task 8 REDs, spec and product
contracts.

### P1-2 — the lock inode has an undocumented physical-workspace identity input

Evidence:

- The semantic artifact key is physical destination-root identity plus canonical
  locus and is claimed to serialize cooperating aliases (`85-94,251-260`).
- The actual inode is always created below the retained workspace's fixed
  `.build/unica/project-discovery/control-v1` (`1946-1968,1994-2020`).
- Therefore the physical workspace containing the control root is an implicit
  third identity component even though it is absent from
  `ArtifactMutationLeaseKey`.
- The design claims path/case/bind views of the same physical destination root
  plus locus reach the same artifact inode (`2017-2020,3030-3032`), while the
  narrower later test text sometimes adds “same physical workspace”
  (`3325-3328`). These claims are not equivalent.
- Current containment rejects symlink/reparse traversal but does not prohibit a
  Unix bind mount as a configured contained source root. Two different physical
  workspaces can therefore expose the same physical extension root and locus,
  yet use different physical control roots.

Impact:

Two cooperating Unica processes can derive the same two-digest key but lock two
different inodes and mutate the same destination artifact concurrently. The key
value equality tests pass while the actual serialization guarantee fails.

Minimal RED:

On Linux, create physical workspaces W1 and W2, bind-mount the same physical
destination source root into each, resolve the same canonical locus, and start
two processes. The accepted contract must choose one explicit outcome:

1. both map to one shared lock inode and one contender receives
   `cfe_artifact_busy`; or
2. both are rejected before authoritative capture with a stable unsupported
   cross-workspace/mount-boundary reason.

It is not sufficient to assert that the computed `ArtifactMutationLeaseKey`
bytes are equal.

Required correction and ownership:

Task 8 lease infrastructure/spec must define the serialization universe. Either
place the artifact lock in a filesystem namespace physically shared by every
allowed alias of the destination root, or explicitly reject internal mount/
cross-workspace destination sharing and delete the broader bind-alias claim.
Task 9 may keep workspace-scoped receipt records, but it must reuse the corrected
artifact boundary. Add native two-process tests; a process-local fake is not
proof.

### P1-3 — Unix filesystems have no supported atomic/lock capability boundary

Evidence:

- Absent on Unix relies on `linkat` plus unlink (or another native no-replace
  primitive), while the lease relies on a persistent file plus `fs2` OS locking
  (`1994-2016,2858-2865`).
- Windows gets a versioned local-NTFS/build allowlist and fails unproven tuples
  before source mutation (`2867-2901`).
- Linux/macOS final verification requires native runners, but no filesystem or
  mount-type allowlist, runtime capability gate, or network/FUSE boundary exists
  (`3455-3457`).
- `fstatfs`/mount identity is already read for identity (`1976-1992`), but the
  design uses it only in the key, not to establish coherent cross-process locks,
  atomic no-replace and durable directory metadata semantics.

Impact:

The same production code can run on NFS/CIFS/FUSE or another unproven Unix
filesystem where advisory-lock coherence, hard-link behavior or durability
differs. A syscall existing, or one happy-path `linkat` succeeding, does not prove
the full lock/rename/fsync contract. The Windows guarantee is scoped honestly;
the Unix guarantee is currently accidental and broader.

Minimal REDs:

1. A fake/real unallowlisted filesystem identity fails before parent/temp/source
   mutation with `cfe_patch_atomic_backend_unsupported`.
2. Each allowlisted Linux/macOS filesystem runs the two-process Busy suite,
   Absent no-replace race, Present cooperative replacement, directory durability
   and every failure seam.
3. Network, FUSE and unknown filesystem identities never silently fall through
   merely because `linkat` or `flock` returned success once.

Required correction and ownership:

Define `unica.unix-contained-atomic.v1` with an explicit supported filesystem/
mount tuple and exact lock/no-replace/durability primitives, or weaken product
support and fail all unproven tuples. Task 8 owns detection and typed failure;
Task 9 reuses the backend; active spec/ADR/package CI must state the same matrix.

## P2 finding

### P2-1 — `NoChange` says “zero persistent effects” although apply may create the control plane

Evidence:

- The global invariant describes `NoChange` as verified-clean with zero
  persistent effects (`122-129`).
- The first applied call can create fixed control directories and a persistent
  lock inode before authoritative capture or handler outcome
  (`1954-1968,1994-2016,2036-2047`). The inode is intentionally never deleted.
- `MutationEffectsV1` and `NoChange::new` account only for source parent/target/
  staging effects (`2640-2692,2704-2735`).

Impact:

The implementation can be correct, but the unqualified invariant is false and
will confuse replay/observability tests. A material-precondition failure on the
first apply can return `NoChange` while having created durable control-plane
state.

Required correction:

Define `NoChange` as zero persistent **source-artifact mutation effects** and
explicitly exclude expected control-plane lease initialization, or add a separate
internal control-plane effect channel. Product diagnostics must never mix those
with grant-authorized source effects. Add the first-call preflight-failure RED.

## Back-propagation and ownership map

| Finding | Task 8 | Task 9 | Task 10 | Earlier prerequisites/spec |
| --- | --- | --- | --- | --- |
| P0-1 staging lifecycle | effect types + atomic writer + REDs | schema/version only | advance/revoke and replay | active spec/ADR/observability |
| P0-2 durability | durability type + platform classification | generic persisted schema boundary | require durable+clean before advance | active spec/ADR and platform matrix |
| P1-1 Form authority | consume only shared complete view | store semantic version/digest | re-prove same view | Task 5B shared registry, Task 6 identity, product tests |
| P1-2 lock universe | lease root/key/inode contract + native tests | reuse exact artifact lease; receipts stay workspace-scoped | preserve artifact-first order | active spec/ADR, topology restrictions |
| P1-3 Unix backend | versioned support gate + native matrix | reuse backend | revoke/nonadvance on unknown | package/CI/support docs |
| P2-1 NoChange scope | wording/type diagnostics | none | do not treat control files as grant effects | spec/observability wording |

## Re-review gate

A v5 review may return READY only after all of the following are immutable and
GREEN:

1. P0 staging-name/object accounting and durability are represented by
   uninhabitable-by-mistake types and failure-injection tests.
2. Task 8 contains no duplicate Form item/event/callType/Action registry and the
   accepted Task 5B shared registry has no open P0/P1.
3. The lock universe chooses and proves one behavior for cross-workspace aliases;
   no test equates key bytes with lock-inode equality.
4. Unix and Windows both have explicit supported backend tuples and native
   no-replace/lock/durability tests.
5. Task 5A has an accepted committed SHA, Task 5B is accepted on that SHA, and
   strict MDClasses namespace, CFE flavor/Own authority and the closed
   EventSubscription registry are GREEN before Task 8 code.
6. Active spec/ADR/skill/product contracts and Tasks 9/10 carry the corrected
   outcome and lock boundaries without claiming production receipt proof in Task
   8.

Until then, the correct action is STOP; implementation would encode unresolved
authority semantics into the shared writer and receipts.
