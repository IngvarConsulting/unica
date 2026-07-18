# Task 4 implementation report

## Result

Implemented contained project-source resolution and immutable content snapshots.
The implementation was committed as:

- `89f390bb077ba20774b206656b4a6f094d1d1aa4`
- `feat: добавить content source snapshots`
- `c544dc2d120270ddcc79b5e8f51da0d7e2bef22c`
- `fix: закрыть финальные snapshot races`

The commit contains only the 16 Task 4 Cargo/code files. Controller-owned
changes in `docs/superpowers/plans`, `spec`, and `tests/ci/test_product_contracts.py`
were deliberately left unstaged.

## RED evidence

The first focused RED run was:

```text
cargo test --locked -p unica-coder source_snapshot -- --nocapture
5 tests: 4 passed, 1 failed
domain::source_snapshot::tests::composite_fingerprint_is_not_a_caller_supplied_placeholder
left == right: both were the caller-supplied sha256:4444... placeholder
```

The first source-map RED run was:

```text
cargo test --locked -p unica-coder project_sources -- --nocapture
7 tests: 5 passed, 2 failed
duplicate case-folded source-set names and absolute roots were accepted
```

Review-driven regressions were also added before their fixes. The combined
source-snapshot RED run had five failures covering bounded verified reads,
false source-changed classification for stable I/O, absent tombstones through
symlinked ancestors, and EDT ancestor containment. The project-source RED run
failed for `ConfigDumpInfo.xml` format evidence and silently dropped EDT
destinations.

## Implementation and decisions

- Kept `SourceFormat`, source identities, manifests, and fingerprint encoders
  pure in the domain; moved YAML and filesystem behavior into infrastructure.
- Added map-wide `ResolvedSourceSelection` validation, canonical destination
  ordering/deduplication, exact role/kind/format validation at both resolver and
  application boundaries, and semantic mapping digests.
- Built source-format-aware manifests from direct Platform XML registration,
  registered object/nested source trees, exact root `Ext` artifacts, and typed
  optional tombstones. `ConfigDumpInfo.xml`, unregistered material, and ignored
  generated corpora do not become authoritative.
- Added strict UTF-8/BOM/namespace-aware XML parsing with direct-child,
  identity, duplicate, and mixed-content validation.
- Added component-wise no-follow containment. Unix uses `openat` identities and
  bounded nonblocking leaf opens; Windows uses relative `NtCreateFile`, retained
  parent handles, reparse/final-path checks, and `FileIdInfo` identity.
- Capture enumerates twice, hashes unique workspace-relative files once across
  the composite snapshot, enforces file/byte/traversal/XML/deadline bounds, and
  revalidates identities and mapping state before acceptance.
- Verified reads check manifest membership first, reapply containment, bound the
  read to the manifest byte length plus one, and distinguish fingerprint changes
  from stable transient I/O.
- Error classification is evidence-aware: observed identity/type/path/content or
  mapping differences are retryable `source_changed_during_capture`; stable I/O,
  unsafe identity unavailability, malformed material, and resource failures keep
  their typed reasons and retry policies.
- EDT configuration analysis resolves and validates the whole requested batch,
  while the use case captures exactly the diagnostic marker manifest, sends no
  destinations to capture, invokes no evidence providers, and returns the
  established blocking readiness report.
- Freshness binds exact source-set and content fingerprint. Epoch remains
  diagnostic: adapter snapshot epochs normalize to the current context, and
  accepted provider records normalize to the snapshot epoch before canonical
  deduplication.

## Review fixes included

- Closed TOCTOU races at the observation/open/read/reopen boundaries, including
  FIFO, symlinked-parent, same-length replacement, and growth races.
- Made registered-subtree traversal name-sorted and deterministic before budget
  and type decisions.
- Counted overlapping composite files/bytes once globally.
- Revalidated absent EDT and optional-material ancestors before accepting a
  tombstone.
- Preserved stable I/O and identity-unavailable classifications instead of
  blanket promotion to source-changed.
- Promoted structural map changes during pre/post capture revalidation while
  preserving stable map I/O.
- Excluded `ConfigDumpInfo.xml` as independent Platform XML evidence for both
  configuration and external source kinds.
- Normalized diagnostic epochs before evidence collision/canonicalization.

The first independent final reviewer verdict was `APPROVED` with no Critical or
Important findings. The later immutable-package review superseded that verdict
with three Important findings. Those findings and their fixes are recorded
below rather than silently treating the earlier approval as current.

## Immutable-package review fixes

### Additional RED evidence

The review regressions were written and observed failing before the fixes:

```text
absent_optional_appearance_after_final_scan_discards_snapshot
failed: unwrap_err() received an accepted SourceSnapshot whose
main/Ext/ParentConfigurations.bin entry was still AbsentOptional after the file
appeared at BeforeFinalIdentityValidation.

edt_absent_marker_appearance_after_final_scan_discards_snapshot
failed: unwrap_err() received an accepted EDT SourceSnapshot whose
edt/DT-INF/PROJECT.PMF entry was still AbsentOptional after the marker appeared
at BeforeFinalIdentityValidation.

final_present_reread_is_bounded_by_captured_length
failed: left SnapshotResourceLimit, expected SourceChangedDuringCapture.

public_map_rejects_a_truly_missing_configured_root
public_map_rejects_dangling_leaf_and_ancestor_symlinks
failed: the public map returned Ok PlatformXml from the configured DESIGNER
format even though the root was missing or reached through a dangling symlink.
```

### Fixes

- Revalidated every unique `AbsentOptional` path after the final mutation hook
  and present-file rereads. A newly present optional file, symlink, reparse
  point, or changed ancestor topology now rejects the whole capture as retryable
  `SourceChangedDuringCapture`.
- Bounded each final present-file reread by that file's captured byte length.
  Growth can no longer consume the global snapshot allowance or surface as a
  4-GiB-style resource-limit failure; it becomes retryable source change.
- Removed the public source-map missing-root exception. Public and typed
  resolution now share the same contained-directory validation and reject
  missing roots plus dangling leaf/ancestor links.
- Added the direct `ConfigDumpInfo.xml`-only assertion for the exact typed
  readiness tuple: `unknown_source_format`, role `Analysis`, non-retryable.
- Updated `external_init_preview_is_path_guarded_and_source_set_typed` because
  its old fixture contradicted the authoritative public-map contract: it
  configured roots that it simultaneously required not to exist. The fixture
  now creates configured roots before discovery. This does not weaken dry-run
  coverage: the assertions still prove that no `Preview.xml`, nested output, or
  other artifact is created.

The immutable-review fixes were committed separately as
`c544dc2d120270ddcc79b5e8f51da0d7e2bef22c`.

### Post-fix GREEN evidence

```text
cargo test --locked -p unica-coder source_snapshot -- --nocapture
36 passed, 0 failed.

cargo test --locked -p unica-coder project_sources -- --nocapture
12 passed, 0 failed.

cargo test --locked -p unica-coder \
  external_init_preview_is_path_guarded_and_source_set_typed
1 passed, 0 failed.

cargo test --locked -p unica-coder
509 passed, 0 failed.

cargo fmt --all -- --check
passed.

cargo clippy --locked -p unica-coder --all-targets -- -D warnings
passed.

python3 tests/ci/test_product_contracts.py
14 passed.

git diff --check && git diff --cached --check
passed.

clean isolated actual contained_fs.rs build for
x86_64-pc-windows-gnullvm with -D warnings
passed.
```

## Initial package verification

```text
cargo test --locked -p unica-coder source_snapshot -- --nocapture
32/32 passed during the focused pass; all expanded snapshot tests also pass in
the final full suite.

cargo test --locked -p unica-coder project_sources -- --nocapture
9/9 passed during the focused pass; the added external ConfigDumpInfo regression
also passes in the final full suite.

cargo test --locked -p unica-coder platform_xml -- --nocapture
7/7 passed.

cargo test --locked -p unica-coder
504 passed, 0 failed.

cargo fmt --all -- --check
passed.

cargo clippy --locked -p unica-coder --all-targets -- -D warnings
passed.

python3 tests/ci/test_product_contracts.py
14 passed.

git diff --check
passed.

isolated actual contained_fs.rs build for x86_64-pc-windows-gnullvm with
-D warnings
passed.
```

## Remaining risk

The Windows containment implementation compiled cleanly against the Windows
Rust target in an isolated crate, but a full `unica-coder` Windows cross-build
and live junction/share-mode runtime test could not run on this macOS host
because native `ring`/SQLite dependencies require a Windows C sysroot/toolchain.
No Task 5 evidence providers or public MCP wiring were added.

## Token usage

The execution environment does not expose an exact per-task token counter.
Therefore an exact token number cannot be reported without inventing one.
