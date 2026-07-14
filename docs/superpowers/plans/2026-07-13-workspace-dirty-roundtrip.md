# Historical implementation record: Persistent Source/Infobase Round-Trip Safety

> **COMPLETED LOCALLY — 2026-07-14:** This historical plan records the
> implementation of issue #76 after the earlier paused checkpoint. Draft PR
> #87 is published; it remains explicitly dependent on PR #86.
> Rust final gates pass: rustfmt, all-target/all-feature Clippy with
> `-D warnings`, `git diff --check`, 393 `unica-coder` tests, and the complete
> Python CI suite (135 passed, 1 expected skip). The original user database was
> not opened, copied, or passed to any command.
>
> The resumed hardening preserves the lifecycle lease in spawned runtime
> descendants, restores standard module roles, makes preview recovery
> read-only, completes CDFI per-write CAS and raw `sourceSet` identity checks,
> and removes Rust warnings. Do not treat the older checked boxes below as
> independently current acceptance evidence; this header and the issue/PR
> record supersede the paused checkpoint.

> **Historical execution record:** code, tests, package metadata, issue #76 and
> its eventual stacked PR are the source of truth after implementation.
>
> **For agentic workers:** keep this checklist current while implementing #76.

**Goal:** Persist exact metadata/module mutation state, make an ordinary runtime
build prove which dirty targets reached the infobase, and prevent partial dump
from overwriting divergent local source unless the caller explicitly forces it.

**Dependency:** `Depends on #73 / PR #86`. This branch is based on exact PR #86
HEAD `6ac39c65ab591744664aef9e128116a2982677b0` and the future PR must remain
stacked on `feat/issue-73-code-patch` until #86 is merged.

**Architecture:** Add a source-sync authority separate from cache state. A
workspace-scoped `fs2` lifecycle lock serializes Unica mutations with build and
dump. Mutation preflight captures raw source manifests, successful apply records
the postimage, and a durable fail-closed repository stores current versus last
synchronized file hashes. Build consumes the pinned runner's JSON step result
and clears only stable targets from source sets that actually ran. Partial dump
uses a shadow Designer source-set and compares its output before any publication
to the working source; default conflicts never expose platform output to source,
while explicit force performs a guarded atomic publication.

**Tech stack:** Rust `unica-coder`, SHA-256 raw-byte manifests, `fs2`, atomic
same-directory JSON/filesystem publication, typed `v8-runner 0.5.1` JSON output,
Python product/skill guardrails, and a new disposable 1C file infobase.

---

## Contract decisions

- The sync authority lives below `.build/unica/source-sync/<workspace-id>/`,
  never in tracked 1C sources and never in `ConfigDumpInfo.xml`.
- `workspace-id` is SHA-256 of the canonical workspace root so a shared
  `UNICA_CACHE_DIR` cannot mix repositories. `workspace_epoch` is not a dirty
  generation and branch switches do not silently clear unfinished state.
- State has `schemaVersion`, optimistic `generation`, canonical workspace
  identity, and target records keyed by source-set plus logical target id.
- File state is `present(sha256:<raw-byte hash>)` or explicit `deleted`;
  synchronized state may additionally be `unknown`. Dirty is derived from the
  current and synchronized manifests, not persisted as an independent flag.
- Module targets track exact BSL bytes including BOM/EOL. Metadata owner targets
  track the descriptor plus their complete owner directory. Platform-generated
  `ConfigDumpInfo.xml` is excluded from target hashes and never synthesized.
- A real `meta.edit` or `code.patch` captures pre/post manifests. Dry-run,
  rejection and exact no-op do not create or advance source-sync state.
- Ordinary build remains ordinary: Unica does not inspect runner `.redb`, forge
  mtimes/CDFI, or silently substitute `--full-rebuild`. The bundled runner
  already SHA-256 scans source. A dirty target is cleared only when its
  source-set step is terminal `ok`, mode is not `skipped`, and a post-build hash
  CAS still matches the requested manifest.
- Failed/partial multi-source build advances only source sets proven successful.
  Missing/skipped steps and concurrent source changes remain dirty and are
  reported, never hidden behind overall process success.
- Safe partial dump supports platform-XML configuration source sets in #76.
  EDT, external source sets, full/incremental dump protection and extensions are
  rejected or explicitly reported as outside this first safety contract.
- A persisted target pins its canonical source-set root. Build and dump fail
  closed if the current default project topology moved that source-set, or if a
  custom build config cannot be proven identical. With any source-sync record,
  typed full/incremental dump is blocked because it bypasses object-level
  shadow/CAS protection.
- Partial dump normalizes and deduplicates `TYPE:NAME` selectors, requires a
  known synchronized baseline by default, and runs the platform against a
  shadow source-set. Shadow output is compared before working-source publication.
- `force` is a Unica-only boolean accepted only by partial dump. It is never
  forwarded to v8-runner. Without it, any local or infobase divergence returns a
  structured conflict with zero working-source writes. With it, the caller
  authorizes the shadow IB version to replace requested source targets after a
  final working-source CAS. Platform-produced `ConfigDumpInfo.xml` is published
  as a separate source-set auxiliary in the same rollback journal, while
  remaining outside target manifests and dirty-state identity.
- Mutation and runtime results expose non-overlapping `requested`, `processed`,
  `skipped`, and `conflicted` arrays with machine-readable reasons, target id,
  source set, owner selector, paths and expected/current hashes.
- Legacy `unica.build.*` commands must not bypass an active source-sync safety
  record; they fail closed with guidance to use typed `unica.runtime.execute`.
- Corrupt state is an explicit blocking error. It is never treated as empty.
- The original database `/Users/korolev/Bases/Trade_11_5_Demo_Unica` is never
  opened, copied or passed to a command. Platform acceptance uses a newly
  created disposable file IB under `/private/tmp` only.

## Task 1: Durable source-sync domain and RED tests

**Files:**

- Create `crates/unica-coder/src/domain/source_sync.rs`
- Create `crates/unica-coder/src/infrastructure/source_sync.rs`
- Modify `crates/unica-coder/src/domain/mod.rs`
- Modify `crates/unica-coder/src/infrastructure/mod.rs`

- [x] Define typed target/file fingerprints, current/synchronized manifests,
      build/dump terminal entries and stable camelCase serialization.
- [x] Resolve configured platform-XML source roots with canonical containment;
      resolve `meta.edit` object owners and `code.patch` modules to source-set,
      logical id, owner selector and complete raw file manifest.
- [x] Persist namespaced state under one lock with schema/generation validation,
      `create_new` staging, flush/sync, atomic replace and parent-directory sync.
- [x] Fail closed on malformed/foreign/newer state and prove restart persistence,
      shared-cache separation, explicit deletions, symlink rejection, atomic
      failures, stale-generation CAS and concurrent writers.

## Task 2: Mutation capture and public affected-target results

**Files:**

- Modify `crates/unica-coder/src/application/mod.rs`
- Modify `crates/unica-coder/src/application/ports.rs`
- Modify `crates/unica-coder/src/infrastructure/native_operations/code.rs` only
  if direct-source resolution cannot reuse the shared resolver
- Modify `crates/unica-coder/src/infrastructure/native_operations/meta.rs` only
  if metadata identity cannot be captured in application preflight

- [x] Acquire one workspace lifecycle lock before real source mutation, capture
      target preimages, invoke the existing guarded writer, then persist only
      real successful changes and release the lock after cache notification.
- [x] Preserve the first synchronized preimage across repeated local mutations;
      update only current hashes and collapse current==synchronized to clean.
- [x] Merge structured `affectedTargets` into `OperationResult.details` without
      replacing the #73 patch details or human stdout.
- [x] Cover `meta.edit` and `code.patch` apply, repeat/no-op, dry-run, rejection,
      sourceDir/sourceSet identity, raw BOM/EOL hashes, application restart and
      failure to persist after the writer already changed source.

## Task 3: Build handshake over pinned runner JSON

**Files:**

- Modify `crates/unica-coder/src/infrastructure/internal_adapters.rs`
- Modify `crates/unica-coder/src/application/mod.rs`
- Modify `crates/unica-coder/src/application/tool_contracts.rs` as needed
- Modify `crates/unica-coder/src/infrastructure/source_sync.rs`

- [x] Invoke build with observable `--json-message`, snapshot selected dirty
      manifests/generation, and parse `data.steps[{source_set,mode,ok}]` without
      inferring success from arbitrary log text.
- [x] Advance synchronized manifests only for stable targets in successful
      non-skipped steps; retain failed, missing, skipped and concurrently changed
      targets and classify each requested target exactly once.
- [x] Return structured requested/processed/skipped/conflicted data for success,
      partial failure, total failure and dry-run; never clear on invalid JSON.
- [x] Block legacy build/load/update bypass when relevant sync state exists and
      test source-set filtering, multi-source partial success, skipped dirty,
      timeout/spawn/exit and post-build hash mismatch.

## Task 4: Shadow partial dump and explicit force

**Files:**

- Modify `crates/unica-coder/src/application/tool_contracts.rs`
- Modify `crates/unica-coder/src/infrastructure/internal_adapters.rs`
- Modify `crates/unica-coder/src/infrastructure/source_sync.rs`

- [x] Accept `force` only for `operation=dump, mode=partial`; keep it out of the
      v8-runner argv and reject unsafe source formats/ambiguous source sets.
- [x] Build a private pinned primary/local config pair and isolated shadow
      source under an owned source-sync transaction; seed exact platform root
      bytes captured under the lifecycle lock and never rediscover mutable
      workspace config during runner execution.
- [x] Run one failure-atomic shadow batch, hash every requested owner/module,
      and classify identical output, local divergence, IB divergence, missing
      baseline/object and runner failure before working-source publication.
- [x] On default conflict discard shadow and prove every working-source byte is
      unchanged. On explicit force perform final source CAS and atomically
      publish only requested owner files plus platform CDFI, with rollback and
      orphan recovery for injected failures/interruption.
- [x] Test duplicate selectors, multi-object conflict, unknown target, source
      deletion/creation, descriptor+module bundles, concurrent editor changes,
      shadow/config cleanup, redacted commands and no `--force` forwarding.

## Task 5: Documentation, review and real acceptance

**Files:**

- Modify `plugins/unica/skills/v8-runner/SKILL.md`
- Modify relevant product/skill tests under `tests/ci`
- Update this checklist and `/Users/korolev/Projects/UNICA_OVERNIGHT_STATUS.md`

- [x] Document affected-target state, ordinary build reconciliation, partial
      dump conflict/force workflow, scope boundaries and recovery guidance.
- [x] Run focused tests, full `cargo test --locked -p unica-coder` (393 tests),
      all-target/all-feature Clippy with `-D warnings`, rustfmt, full Python CI
      (135 passed, 1 expected skip on Python 3.12), Python compilation, plugin
      manifest validation and `git diff --check`.
- [x] Run the personal Rust review skill and an independent runtime/security
      acceptance review; fix every blocking/high finding and repeat gates. The
      final review found no remaining blocking or high-severity issue.
- [x] In a new disposable file IB, prove `meta.edit -> normal build -> partial
      dump` preserves the property and `code.patch -> normal build -> partial
      dump` preserves BSL bytes/BOM; create a different IB version, prove default
      conflict leaves source exact, then prove explicit force publication.
- [x] Commit `a730495`, push and open Draft PR #87 with `Depends on #73 / PR
      #86` and `Closes #76`. GitHub cannot use a fork-only branch as an upstream
      PR base, so #87 targets `main` and stays Draft until #86 merges; then
      rebase it onto `main`, force-push and recheck CI before marking ready. Do
      not select another issue in this execution run.
