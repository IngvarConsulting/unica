# Safe single-file publisher for Unica

- Date: 2026-07-20
- Status: approved on 2026-07-20; implementation authorized
- Related work: issue #74, pull request #163

## Context

Pull request #163 changed the shared `write_utf8_bom` helper to publish through a sibling staging file. That avoids truncating the target before all replacement bytes have been written, but the helper is used by dozens of call sites and does not yet define a safe mutation contract. Igor's review identified the concrete gaps: target permissions are not preserved, read-only targets are overwritten, symbolic links are replaced rather than handled explicitly, staging files can survive write or sync failures, and the unrelated `AGENTS.md` change does not belong in the pull request.

`CompileTransaction` already contains stronger versions of several needed mechanisms: create-only publication, preimage checks, target locks, permission preservation, staging collision retries, cleanup, validation, and rollback. Keeping a second, weaker implementation in `common.rs` would let their guarantees drift.

This design extracts one internal publication primitive and uses it from both a normal edit path and `CompileTransaction`. It is the first independently mergeable slice of the broader writer-mutation contract tracked by issue #74.

## Goals

The slice must:

- publish a complete replacement without exposing a partially written target;
- distinguish create-only publication from replacement in the type system;
- reject stale replacements by comparing the current target with the caller's expected preimage;
- reject read-only files, links/reparse points, non-regular files, and files with multiple hard links;
- preserve the existing target's portable permissions on replacement;
- attempt staging cleanup on every failure path and never hide a cleanup failure;
- use the existing platform facade for the final atomic filesystem operation;
- return a typed effect: `Created`, `Replaced`, or `Unchanged`;
- share the primitive with `CompileTransaction` and migrate one ordinary writer, `cf-edit`, as the first consumer;
- remove the unrelated `AGENTS.md` change from pull request #163.

## Non-goals

This slice does not:

- migrate every current `write_utf8_bom` call site;
- implement all of issue #74's later contract, including explicit EOL policy, public expected hashes, byte-range edits, or a repository-wide mutation API;
- preserve ACLs, ownership, extended attributes, alternate data streams, or other metadata beyond portable `std::fs::Permissions`;
- guarantee recovery after process termination, kernel crash, or power loss;
- synchronize parent directory entries as a crash-durability guarantee;
- defend against an adversary that can continuously swap path components between checks and the final filesystem call;
- move semantic XML/BSL validation or multi-file rollback into the single-file publisher.

The promised property is **failure atomicity for reported errors**, not power-loss durability. The concurrency model is cooperative callers plus best-effort detection of external changes.

## Considered approaches

### Keep strengthening `write_utf8_bom`

This is the smallest textual change, but it silently changes all of its many consumers at once. The helper also cannot express whether a missing target is expected, which preimage was edited, or whether an unchanged result should be reported. It would continue duplicating `CompileTransaction` behavior.

### Move every writer to `CompileTransaction`

This maximizes reuse but couples simple one-file edits to a multi-file metadata compilation abstraction. It would make pull request #163 much larger and would mix transaction planning, semantic validation, and rollback with the narrower publication concern.

### Extract a shared publisher and migrate incrementally

This is the selected approach. A new internal `single_file_publisher` module owns one-file safety policy. `CompileTransaction` reuses its staging and commit building blocks while retaining multi-file orchestration and rollback. `cf-edit` becomes the first ordinary edit consumer. Other writers remain unchanged until subsequent issue #74 slices can migrate them with focused tests.

## Architecture

### Module boundary

Add:

`crates/unica-coder/src/infrastructure/native_operations/single_file_publisher.rs`

The module is `pub(crate)` and is not an MCP or application-layer API. OS-specific operations stay behind `infrastructure::platform::filesystem`; the new module must not introduce direct Windows FFI or OS-specific replacement branches.

The filesystem facade gains only the primitives the policy layer cannot implement portably: atomic no-clobber installation, hard-link-count inspection, and any host-specific permission/reparse handling. Existing `replace_file_atomically`, `metadata_is_link_or_reparse_point`, `path_lock_identity`, and `prepare_file_for_removal` remain the canonical platform boundary.

The high-level conceptual API is:

```rust
pub(crate) enum PublishMode {
    CreateOnly,
    ReplaceExisting { expected_preimage: Vec<u8> },
}

pub(crate) enum PublishEffect {
    Created,
    Replaced,
    Unchanged,
}

pub(crate) struct PublishReport {
    pub(crate) effect: PublishEffect,
    pub(crate) cleanup_warnings: Vec<CleanupWarning>,
}

pub(crate) struct PublishRequest {
    pub(crate) target: PathBuf,
    pub(crate) replacement: Vec<u8>,
    pub(crate) mode: PublishMode,
}

pub(crate) fn publish(request: PublishRequest)
    -> Result<PublishReport, PublishError>;
```

The implementation may use borrowed buffers or `Arc<[u8]>` to avoid unnecessary copies, but the public internal semantics above must remain explicit. It must not collapse the modes into booleans or infer them from target existence. The report keeps the required typed effect while allowing an already-committed publication to report post-commit cleanup trouble as a warning instead of returning a misleading failure.

For `CompileTransaction`, the module also exposes crate-private preparation and commit building blocks. A prepared publication owns its staging cleanup guard and the target snapshot needed for the immediate pre-commit recheck. `CompileTransaction` can prepare all files, run its existing semantic validation, and then commit them under its existing ordered lock and rollback orchestration. The simple `publish` function is a one-file composition of the same steps.

### Typed errors

`PublishError` is an internal typed error wrapper around a `PublishErrorKind` enum and implements `Display` and `Error`. The wrapper carries cleanup warnings without duplicating that field across every error variant. The kind distinguishes at least:

- invalid or missing parent/filename;
- target already exists in `CreateOnly` mode;
- target missing in `ReplaceExisting` mode;
- link/reparse-point target;
- non-regular target;
- read-only target;
- multiple-hard-link target;
- stale preimage or metadata observed before commit;
- exhausted staging-name collisions;
- I/O failure with a phase such as inspect, stage, write, sync, validate, commit, or cleanup.

Adapter boundaries may convert the typed error to their existing string result. Internal code and tests must not classify publication failures by matching message text.

### Target locks

The publisher reuses the same path identity, process-local mutex, and advisory lock-file scheme used by `CompileTransaction`. Lock identity is derived by canonicalizing the existing parent directory and appending the target filename; it must not canonicalize through the target and accidentally follow a link. A one-file publish holds its lock from the first authoritative target inspection through commit or cleanup. A multi-file transaction acquires locks in stable sorted order and keeps them through rollback/finalization.

The locks coordinate Unica processes that follow this protocol. They do not prevent unrelated programs from modifying the target, so the preimage and metadata rechecks remain mandatory.

## Publication contract

### `CreateOnly`

- The target must be absent during initial inspection and immediately before commit.
- Existing files, directories, links, and reparse points are never replaced.
- The staged file is installed with a no-clobber operation. On supported filesystems this can be the existing hard-link-then-unlink technique; the operation is exposed through the filesystem facade so native-operation code does not select an OS implementation.
- If another process creates the target first, publication returns `AlreadyExists`, preserves that target, and removes the stage.
- Successful publication returns `Created`.

The new file retains the normal permissions selected when its staging file is created. On Unix, an empty stage may be created with the process-default mode, that mode is captured, and the stage is tightened to owner-only before the first content byte is written; the captured final mode is restored only after the complete content is synced. This preserves current create behavior without exposing partial content under the final mode. Platform ACL behavior is inherited from the parent and is outside this slice's portable-permissions guarantee.

### `ReplaceExisting`

- The target must exist and be a regular file.
- `symlink_metadata` and the platform reparse-point helper are used so inspection does not follow a link.
- A read-only target is rejected even if the current user could clear the flag or change its mode. The publisher does not override the caller's filesystem policy.
- A target with more than one hard link is rejected. Replacing one directory entry would otherwise leave the other names pointing at the old inode and violate an edit-in-place expectation.
- The current bytes must equal `expected_preimage` before staging and again immediately before commit.
- Target kind, read-only state, hard-link count, and portable permissions are inspected again immediately before commit. A conflicting change fails closed.
- The replacement stage receives a clone of the target's portable permissions before commit.
- If the verified current bytes already equal the replacement, no stage is created and the result is `Unchanged`.
- Otherwise the platform facade atomically replaces the directory entry and the result is `Replaced`.

A replacement by another program that has identical bytes and indistinguishable portable metadata may not be detected. Closing that final path-based TOCTOU window requires handle-relative or platform-specific identity/CAS APIs and is outside the cooperative threat model of this slice.

### Staging lifecycle

1. Resolve a usable parent and filename without canonicalizing through the target itself.
2. Acquire the target's publication lock.
3. Inspect and validate the mode-specific target state.
4. Return `Unchanged` immediately when applicable.
5. Create a sibling stage with `create_new(true)`. A process id plus monotonic sequence supplies candidate names; bounded retries handle collisions.
6. Arm an RAII cleanup guard immediately after successful creation. Before writing the first byte, capture the stage's process-default creation permissions and tighten its portable permissions where the platform supports that operation.
7. Write all replacement bytes, flush, and `sync_all` the staged file.
8. Apply the final portable permissions, sync the permission change, and verify the staged bytes exactly. A replacement uses the target snapshot; a create restores the captured process-default creation permissions.
9. Re-inspect the target and repeat the mode/preimage checks immediately before commit.
10. Commit with the platform facade's replacement or no-clobber primitive.
11. Disarm the guard only after the stage has been consumed or removed successfully.

Every error after stage creation passes through the guard. Cleanup uses `prepare_file_for_removal` for Windows read-only behavior. Before commit, a cleanup failure is attached to the returned primary error rather than replacing or hiding it. After a successful no-clobber commit, failure to unlink the second staging name is reported in `PublishReport.cleanup_warnings`, because the target already contains the requested bytes and returning `Err` would falsely imply that no commit occurred. Staging cleanup is also attempted during unwinding, but the API does not claim recovery from process abort or power loss.

The publisher validates exact staged bytes before the atomic directory-entry operation. Format-specific and post-publication validation remain with the caller. In particular, `CompileTransaction` keeps XML validation, post-publication checks, backup handling, and rollback of a partially committed multi-file transaction.

## Integration plan

### `CompileTransaction`

Refactor its duplicated stage-name generation, collision retry, stage writing/sync, target policy checks, and RAII cleanup into the shared publisher building blocks. Preserve these transaction-specific responsibilities:

- planning multiple creates and registrations;
- canonical child registration and byte diffs;
- ordered acquisition of all relevant locks;
- XML validation before and after publication;
- backup tracking and rollback across multiple paths;
- commit reports and cleanup warnings.

Create-only files use `CreateOnly`. Registration targets use `ReplaceExisting` with their captured original bytes. Existing behavior and reports remain stable.

### First ordinary writer: `cf-edit`

`cf-edit` is the first non-compile consumer because it edits the same `Configuration.xml` registration target that compile transactions can update.

At read time it retains the exact original bytes in addition to the decoded text used by existing edit logic. When `config_changed` is true, it encodes the replacement with exactly one UTF-8 BOM and calls `publish` in `ReplaceExisting` mode with the raw original bytes as `expected_preimage`. When no logical change is produced, it does not call the publisher. Its MCP-visible success/error shape stays unchanged except that unsafe targets and concurrent changes now fail explicitly.

The generic `write_utf8_bom` helper is not redirected to the new publisher in this slice. Doing so would migrate all callers implicitly. Its PR #163-local `atomic_replace` implementation and tests are removed or restored so there is only one newly introduced safe publication path.

This migration guarantees the mutation contract only for the `Configuration.xml` publication itself. A `cf-edit` request that also writes panel or home-page artifacts is still a multi-file operation and is not claimed to be transactionally atomic by this slice.

### Pull-request hygiene

Remove the PR's unrelated `AGENTS.md` workflow edit. Keep issue #74's full mutation-contract work separate from the narrow publisher slice.

## Test strategy

Implementation follows red-green-refactor. Tests are added before production behavior for each contract branch.

### Publisher unit tests

- `CreateOnly` creates the requested exact bytes and returns `Created`.
- `CreateOnly` never overwrites a target that already exists or appears before commit.
- `ReplaceExisting` publishes exact bytes and returns `Replaced`.
- An identical replacement returns `Unchanged` and creates no staging artifact.
- A stale expected preimage is rejected before staging.
- A target changed after staging is rejected before commit and remains untouched by Unica.
- Unix mode `0600` is preserved after replacement.
- A read-only target such as Unix mode `0400` is rejected and remains byte-for-byte and permission-for-permission unchanged.
- A symbolic link is rejected without changing either the link or its referent.
- A Windows reparse point is rejected where the platform test environment supports creating one.
- A directory and other non-regular target are rejected.
- A target with multiple hard links is rejected without changing either name.
- Staging-name collision retries do not overwrite the colliding file.
- Exhausted collision retries return a typed error.
- Injected write, sync, permission, staged-validation, recheck, and final-commit failures leave the original target unchanged and remove the stage.
- Cleanup failure is reported together with the primary failure.
- Cleanup failure after a successful no-clobber commit returns the committed effect plus a cleanup warning, not a false `Err`.

Failure injection is provided through a small test-only filesystem-operations seam or narrowly scoped failpoints. Tests must exercise real RAII cleanup paths rather than only call cleanup helpers directly.

### Integration and regression tests

- Existing `CompileTransaction` create, registration, validation, contention, rollback, BOM, and EOL tests continue to pass against the shared primitive.
- A compile transaction and `cf-edit` contend through the same lock identity and one receives a stale-preimage failure instead of losing an update.
- `cf-edit` preserves exact source conventions already covered by its tests while publishing through `ReplaceExisting`.
- `cf-edit` exposes clear failures for read-only, link/reparse, hard-linked, and concurrently changed `Configuration.xml` targets.
- Platform replacement tests cover Unix and Windows facade behavior without leaking OS-specific code into `native_operations`.

### Verification

Run focused tests while developing, then:

- format and lint the affected crate;
- run the full `unica-coder` test suite;
- run repository platform-boundary checks;
- run `cargo clippy` with warnings denied for the affected workspace scope;
- review the Rust safety invariants explicitly: exhaustive enums, typed errors retaining
  `io::Error` sources, no runtime-input panics, cfg boundaries, single-consumption
  ownership states, stable lock ordering, post-commit warning semantics, and localized
  Windows `unsafe` blocks with safety comments;
- inspect `git diff --check` and confirm the PR contains no unrelated `AGENTS.md` change.

## Delivery and GitHub updates

After implementation and verification:

- rewrite issue #74 as the parent mutation-contract epic and record this publisher as its first completed child slice;
- update pull request #163's description to state the exact guarantees and deferred work;
- reply to Igor's review points with the corresponding code/tests;
- push the focused branch only after local verification passes.
