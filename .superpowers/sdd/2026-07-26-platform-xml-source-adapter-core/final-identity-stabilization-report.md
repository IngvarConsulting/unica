# Platform XML identity stabilization

## Limits

- `512` captured files.
- `8 MiB` maximum bytes per captured file.
- `64 MiB` maximum bytes across a capture.
- File metadata is checked before allocation; reads use a bounded handle and
  fail with typed `resource_limit` rather than `fs::read` allocation.
- Symlinks are rejected for target components, descriptors, companions, and
  recursively captured root content.

## RED

- A descriptor target previously captured its plural parent recursively, so a
  sibling could change the revision and an unrelated oversized sibling could
  make the target fail.
- Identical `Items.xml` and `Orders.xml` bytes previously produced the same
  revision candidate despite being different navigation targets.
- A Unicode configured source-set name could be used as a path-like logical
  identity instead of a deterministic opaque source id.
- Probe descriptors and ready envelopes could echo a foreign source identity
  or revision without being bound to the captured session.
- The meta-info skill documented the nonexistent serialized key `valueType`.

## GREEN

- A non-root Platform XML target captures only its descriptor, exact companion
  directory, and authorized source-root `Configuration.xml` plus
  `Ext/ParentConfigurations.bin`; root `Configuration.xml` capture is an
  explicit bounded whole-source mode.
- `TargetIdentity` is an opaque digest of the normalized source-root-relative
  descriptor path. Revisions include it and the captured read set; cache
  admission retains the validated `SourceBinding`, including target identity.
- `source_id_for_configured_source_set` is the single logical-id helper:
  safe `main` remains `workspace:main`; Unicode, unsafe, and reserved names
  become collision-domain-separated `workspace:encoded-<sha256>` values.
- `CapturedSourceSession` exposes immutable `SourceBinding` containing source
  id, family, format, target identity, and revision. Registry validation
  rejects foreign probe descriptors, ready snapshots, nodes, relations, and
  cursors.
- The certification path bootstraps two valid sibling targets in one Unicode
  source set and proves the first target's object reference and cursor remain
  live after the second bootstrap. The provider separately proves exact
  identical sibling descriptor bytes yield different target identities and
  revisions.
- Skill prose uses serialized `type` and `valueState`, never `valueType`.

## Verification

- `cargo fmt --all`
- Focused Rust suites: provider `12`, registry `17`, probe `17`, meta `34`,
  navigation `16`, typed result `1`, projector `19`, certification `5`, and
  application `96`: `217/217` passed.
- `python3.12 -m unittest -q tests/ci/test_unica_skills.py`: `31/31` passed.

## Boundary note

Two root descriptors with byte-identical `<Name>` values cannot both be valid
as `Items.xml` and `Orders.xml`: the decoder deliberately verifies descriptor
filename against native identity. The provider test covers byte-identical
target/revision separation; the native cache/continuation test uses two valid
sibling descriptors to preserve that invariant.

## Fix Round 1

- Registry binding validation now walks only typed semantic identity-bearing
  navigation fields. It ignores ordinary property structures and diagnostic
  JSON keys named `sourceId` or `snapshotRevision`, and enforces a depth limit
  of `64` plus a bounded typed-item count before recursive descent.
- The second provider pass retains no duplicate snapshot bytes. It streams each
  expected read-set file through a fixed `64 KiB` buffer and compares bounded
  length and SHA-256 evidence against the retained first snapshot. Peak capture
  memory is at most `64 MiB` retained source bytes plus the fixed buffer and
  metadata overhead.
- Both first capture and verification cap every read at
  `min(8 MiB, aggregate bytes remaining)` plus one sentinel byte. Checked
  accounting rejects stale growth with `resource_limit` without another large
  allocation or read.
- Pre/post regular-file and symlink checks remain in place. The accepted threat
  boundary remains hostile mutation inside the provider's two-pass capture
  window; no platform-specific `openat2`-style dependency was introduced.
- Verification: `cargo fmt --all`; the focused Rust matrix passed `217/217`,
  and `tests/ci/test_unica_skills.py` passed `31/31`.
