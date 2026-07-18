### Task 4: Contained project source resolution and content snapshots

**Files:**
- Modify: `crates/unica-coder/src/domain/project_sources.rs`
- Modify: `crates/unica-coder/src/domain/source_snapshot.rs`
- Modify: `crates/unica-coder/src/domain/mod.rs`
- Create: `crates/unica-coder/src/infrastructure/source_snapshot.rs`
- Modify: `crates/unica-coder/src/infrastructure/mod.rs`
- Test: both source modules

- [ ] **Step 1: Write failing containment and fingerprint tests**

Cover: duplicate source-set names; configured absolute/outside roots; symlink
escape; same-length same-mtime byte change; deterministic path ordering;
mapping/name/kind/format changes; composite configuration+extension snapshot;
analysis plus sorted/deduplicated multiple destination snapshots; generated and
ignored-corpus exclusions; file/byte/time bounds; unreadable material file;
concurrent file mutation during hashing; Unknown/Invalid/EDT format eligibility.

```rust
#[test]
fn content_change_with_unchanged_len_and_mtime_changes_fingerprint() {
    let before = fixture.snapshot().unwrap();
    fixture.replace_same_len_and_restore_mtime("CommonModules/X/Ext/Module.bsl");
    let after = fixture.snapshot().unwrap();
    assert_ne!(before.fingerprint, after.fingerprint);
}

#[test]
fn composite_snapshot_binds_analysis_and_destination() {
    let a = fixture.snapshot_pair("main", "ExtensionA").unwrap();
    let b = fixture.snapshot_pair("main", "ExtensionB").unwrap();
    assert_ne!(a.composite_fingerprint, b.composite_fingerprint);
}
```

- [ ] **Step 2: Run and verify RED**

Run: `cargo test --locked -p unica-coder source_snapshot -- --nocapture`

- [ ] **Step 3: Harden source-map identities**

Canonicalize source roots under the workspace, reject duplicate names and
escapes, and return a typed `ResolvedSourceSet`. Auto-select only one eligible
set; multiple eligible sets without `sourceSet` are an operation error.

- [ ] **Step 4: Implement bounded content manifests**

Hash mapping identity, sorted workspace-relative regular-file paths, and each
file's SHA-256. Build a source-format-aware manifest from Platform XML
registration plus registered metadata files and their contained source
subtrees; do not traverse unrelated workspace files when a source root is `.`.
Hash mapping configuration separately. Never read `docs/research`, `docs/its`,
or follow symlinks/reparse escapes. Use server-owned deterministic file/byte
budgets `maxFiles=200000` and `maxBytes=4GiB`; `maxElapsed=120s` is a safety
abort that discards the entire authoritative snapshot rather than selecting a
timing-dependent prefix. Re-stat/open files around reads; concurrent identity,
size, or metadata change makes the snapshot unavailable/retryable. Preserve an
in-memory path-to-hash manifest for exact pre/post diffs. Exclude `.git`,
`.build`, `target`, and `dist` components inside registered subtrees.

- [ ] **Step 5: Re-run and commit**

Run: `cargo test --locked -p unica-coder source_snapshot -- --nocapture`

```bash
git add crates/unica-coder/src/domain crates/unica-coder/src/infrastructure
git -c commit.gpgsign=false commit -m "feat: добавить content source snapshots"
```

## Controller decisions and non-negotiable invariants

- Keep domain identities pure. Filesystem/YAML resolution belongs in
  infrastructure and implements the existing `ProjectSourceResolverPort`;
  `SourceFormat` has one authoritative definition.
- Model a source-set identity as name, kind, format, contained
  workspace-relative root, and mapping digest. The mapping digest must bind the
  effective source-map configuration; it is not a placeholder.
- Reject absolute `basePath`/`path`, `..`, empty components, non-UTF-8 material
  paths, duplicate names (including case-fold collisions), two identities that
  canonicalize to the same root, missing/non-directory roots, and any symlink
  or Windows reparse-point component. Never convert an outside path back into
  an apparently relative one.
- Auto-selection succeeds only when exactly one eligible analysis source exists.
  Platform XML configuration/extension is authoritative; an EDT configuration
  is diagnostic-only as resolved below. Unknown, Invalid, external processor,
  external report, and every unsupported destination fail typed readiness.
- The sole relative root `.` is the canonical identity for a source-set rooted
  at the workspace itself. Reject embedded `./`, `..`, empty explicit YAML
  `basePath`/`path`, and ambiguous aliases; do not reject the canonical sole
  `.` representation.
- Resolve the requested analysis source-set and every deduplicated destination
  name from one immutable mapping read. Separate resolver calls must not be able
  to combine source identities from different versions of `v8project.yaml`.
- Introduce one contained, snapshot-aware reader reusable by Task 5 providers.
  `read_verified(snapshot, path)` must reapply containment and compare bytes to
  the manifest digest. Any mismatch is retryable `source_fingerprint_mismatch`;
  it must never become complete/no-match evidence.
- Build Platform XML manifests from `Configuration.xml` direct
  `ChildObjects` registration and the shared `discovery_registry`: include each
  registered `<Directory>/<Name>.xml` and its contained source subtree only.
  Do not recursively walk the whole workspace when the source root is `.`.
  `ConfigDumpInfo.xml`, unregistered decoys, `docs/research`, `docs/its`,
  `.git`, `.build`, `target`, and `dist` are non-authoritative/generated and
  excluded.
- Include the versioned exact registry of configuration-root `Ext` artifacts.
  Within
  registered object trees, nested Forms/Templates/Commands and similar
  collections are registration-aware; a recursive directory walk must not make
  an unregistered nested decoy authoritative merely because it is beneath a
  registered object's directory.
- Include optional root `Ext/ParentConfigurations.bin` and bind its
  presence/absence/content (an optional-path tombstone or equivalent
  `read_optional_verified` contract) so `SupportStatePort` can consume support
  evidence from the same snapshot. A concurrent appearance after an absent
  snapshot is a mismatch, not `not_under_support`. Share one typed
  `Configuration.xml` registration parser with Task 5 providers; manifest
  selection and evidence catalog must not interpret registrations independently.
- Treat malformed registration, missing registered object files, unreadable or
  special material files, registration/path-set changes, and concurrent file
  replacement/write/add/remove as whole-snapshot failure. Capture file identity
  before and after reading (device/inode on Unix; volume/file identity on
  Windows), not only length/mtime.
- Enforce 200000 files, 4 GiB, and an injected 120 s deadline before accepting
  the snapshot. Boundary values pass; boundary+1 fails. Tests for time limits
  use an injected clock/deadline rather than sleeping.
- Fingerprints are versioned and domain-separated. A source fingerprint binds
  the full source-set identity and sorted `(relative path, byte length, content
  SHA-256)` entries; it excludes mtime, epoch, and filesystem iteration order.
  Composite identity binds roles plus every full source identity/fingerprint;
  mutation snapshots are canonically sorted and deduplicated.
- Preserve the immutable manifest on `SourceSetSnapshot` so exact pre/post
  diffs and verified reads do not rescan an unconstrained tree. Snapshot capture
  must implement the existing `SourceSnapshotPort` for analysis plus sorted,
  deduplicated destinations.
- Smart constructors recompute and validate source and composite fingerprints
  from immutable manifests/full identities. A caller cannot forge a snapshot by
  supplying an arbitrary syntactically valid SHA-256 string.
- Enumeration is verified as a whole: scan the authoritative path set before
  reads, rederive it after reads, and finally revalidate all file identities.
  This detects concurrent add/remove and registration changes in addition to
  per-file replacement/write races. Use deterministic injected race hooks.
- Evidence execution must have a snapshot-bound verified-read capability (or an
  equivalent explicit snapshot argument) so Task 5 cannot silently reopen
  unconstrained live paths. Provider facts' `Freshness` identities and hashes
  must be checked against the linked snapshot before promotion.
- Containment must apply at open time, not only during an earlier `lstat` or
  `canonicalize`. Prefer component-wise no-follow handles (`openat`/equivalent)
  and verify Windows reparse/final-handle identity. If the platform API cannot
  prove no-follow semantics, fail closed; never downgrade to length/mtime.
- Reuse or extract a canonical hashing primitive without changing Task 2's
  already-tested discovery IDs. If extraction is risky, keep the snapshot
  encoder independently versioned and add golden/permutation tests.

## Required test matrix

In addition to the tests above, cover root aliases, exact containment at every
path component, symlink/reparse roots and descendants, same-length/same-mtime
content replacement, deterministic ordering, mapping/name/kind/format changes,
registered versus unregistered changes, generated/ignored exclusions,
file/byte/deadline boundaries, unreadable/special/non-UTF-8 material files,
concurrent replace/write/add/remove, malformed/missing registered objects,
analysis plus multiple destination ordering/deduplication, and all unsupported
source formats/kinds.

## Resolved audit decisions

- Preserve the accepted EDT behavior: a configuration analysis source with an
  EDT marker gets a complete diagnostic manifest over exactly the four
  `EDT_DIAGNOSTIC_MARKERS_V1` paths (present or typed absent tombstone, at least
  one present), no recursion or destinations. The use case invokes no evidence
  ports and returns the exact blocking skipped/inconclusive
  `source_readiness` check plus unknown proposal verdicts and
  `unsupported_source_format` receipt blocker. EDT extension, Unknown, Invalid,
  and external kinds fail typed readiness as specified in the active spec.
- `ResolvedSourceSelection` owns one map-wide digest and rejects mixed-digest
  identities. Mutation destinations are Platform XML extensions only.
- `DiscoveryError::SourceReadiness` carries reason, analysis/destination role,
  retryability, and source-set for future typed public mapping.
- Only source-root `Ext` uses exact `SOURCE_ROOT_EXT_ARTIFACTS_V1`; it includes
  the application/interface/home-page files listed in the active spec. Every
  registered object/form/template/command `Ext` is a bounded authoritative
  regular-file subtree. Nested collections are reached only from direct
  registrations; include Commands and a Role Rights.xml regression.
- Check validation explicitly allows `ProjectSourceResolverPort` only for
  `code=source_readiness`; evidence providers remain exactly the six ports.

## Delivery protocol

- Follow strict RED -> GREEN -> REFACTOR and record the exact RED failure.
- Run focused tests, full `unica-coder` tests, `cargo fmt --check`, and
  `cargo clippy --locked -p unica-coder --all-targets -- -D warnings`.
- Commit only Task 4 changes with message
  `feat: добавить content source snapshots`.
- Write the implementation report, including decisions, test commands/results,
  commit SHA, and remaining risks, to `.superpowers/sdd/task-4-report.md`.
