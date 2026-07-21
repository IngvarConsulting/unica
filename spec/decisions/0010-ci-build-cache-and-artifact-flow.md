# ADR-0010: CI build cache and artifact flow

- Status: `accepted`
- Date: `2026-07-20`

## Context

The release workflow builds `unica` and `unica-bootstrap` in separate Cargo
target directories on the same platform runner. It then uploads a complete
`unica-tools-*` bundle, downloads that bundle in a separate Linux job to create
the runtime archive, and downloads both complete runtime and tools artifacts in
`package-thin` even though thin packaging needs only runtime metadata and the
three bootstrap binaries.

The reference PR run `29716722998` stored 464.4 MiB of artifacts: 233.3 MiB of
tool bundles and 226.5 MiB of runtime bundles. The duplicate artifacts do not
strengthen the release contract because the platform runner already has every
input required to create and verify its deterministic runtime archive.

## Decision

### Cargo build

1. Each platform build uses one target-specific Cargo target directory for all
   workspace binaries built on that runner.
2. `unica` and `unica-bootstrap` are selected in one mandatory
   `cargo build --locked` invocation. `--locked` keeps the dependency resolution
   equal to the `Cargo.lock` content used in the cache key. A restored cache
   accelerates this command but never replaces it.
3. The Cargo target directory is cached with a key containing runner OS, Unica
   target, the resolved Rust toolchain cache key, and the `Cargo.lock` hash. The
   workflow does not use prefix `restore-keys`, so an exact hit and a miss remain
   distinguishable.
4. Cargo target directories and cache contents are never uploaded as workflow
   or release artifacts.
5. Every platform build reports its target, cache outcome (`exact-hit`, `miss`,
   or `error`), and mandatory Cargo build duration in seconds in the GitHub
   Actions job summary. Cold and warm runs therefore use the same build path and
   measurement. Cache hit rate for a full run is the number of exact hits divided
   by the three platform cache restores. A run with any cache error is reported
   separately and is not valid cold or warm performance evidence.

### Runtime packaging

The platform build job creates and verifies the complete tool bundle locally,
smokes the packaged Unica MCP, and invokes `package-unica-runtime.py` before the
runner is released. It then reopens the target's generated archive and verifies
the archive checksum, file set, member checksums, executable modes, and zeroed
timestamps against its metadata. This single-target verification runs before
the archive is uploaded or discarded. Tag publication retains the aggregate
three-target verification after the published assets are downloaded again.
This preserves deterministic archive creation while removing the intermediate
`package-runtime` job and the `unica-tools-*` artifact family.

The resulting data crosses job boundaries as three independently owned artifact
classes:

- `unica-runtime-metadata-<target>` contains only
  `unica-runtime-<target>.json` at the artifact root;
- `unica-bootstrap-<target>` contains exactly
  `bootstrap/bin/<target>/unica-bootstrap[.exe]`, preserving the layout consumed
  by thin packaging;
- `unica-runtime-<target>` contains only the publishable
  `unica-runtime-<target>.tar.gz` at the artifact root.

Runtime metadata and bootstrap artifacts are uploaded for every package
pipeline and retained for one day. Pull-request and `workflow_dispatch` package
pipelines upload only the Linux runtime archive consumed by release assessment.
Tag pipelines upload all three runtime archives for publication and
byte-for-byte verification. These workflow artifacts are intermediate and use
one-day retention because the release assets become the durable tag output.

`package-thin` downloads only the runtime metadata and bootstrap artifact
families. `unica-thin-marketplace` uses an explicit `retention-days: 90` because
manual marketplace staging and promotion retrieve it by `source_run_id` after
the producing workflow has completed.

### Failure behavior

- A cache miss is an observable cold build, not an error.
- Cargo cache restore and save are best-effort. A restore failure is recorded as
  `error` and does not stop the mandatory Cargo build; a save failure is visible
  in the job log and does not invalidate an otherwise verified build. Neither
  failure may bypass package validation, smoke, or deterministic archive
  verification.
- Missing metadata, bootstrap binaries, or a required runtime archive remains a
  hard artifact/download failure.
- Tag publication still requires all macOS, Linux, and Windows archives and
  metadata, followed by published-byte verification and thin-plugin smoke on
  all supported hosts.

## Verification

Contract tests must prove that:

- the build helper issues one `cargo build --locked` for `unica` and
  `unica-bootstrap` against one target directory;
- the workflow cache key includes OS, target, toolchain, and `Cargo.lock`;
- the workflow uses no prefix restore key, distinguishes exact hit, miss, and
  restore error, and always executes the build after cache restoration;
- no `unica-tools-*` artifact or separate `package-runtime` job remains;
- thin packaging consumes only runtime metadata and bootstrap artifacts;
- bootstrap artifacts preserve the
  `bootstrap/bin/<target>/unica-bootstrap[.exe]` payload layout;
- intermediate artifacts use one-day retention while
  `unica-thin-marketplace` uses 90-day retention;
- pull requests and manual runs upload only the Linux runtime required by
  downstream assessment, while tag runs upload and publish all targets;
- each platform verifies its freshly packaged archive and metadata pair before
  upload or disposal, while tags also verify the complete published matrix;
- package, bootstrap smoke, release assessment, deterministic archive, and
  published-asset contracts remain connected to the stable `Unica CI` gate.

The implementation PR identifies three full workflow runs: the pre-change
baseline, an optimized cold run with no matching cache key, and a warm rerun of
the same tree with the same key. For each optimized run it records per-target
cache outcome and Cargo duration, exact hits out of three attempts, wall time,
aggregate runner time, every artifact size, total upload/download volume, and
the volume downloaded by `package-thin`. Runs with cache errors are diagnostic
only and must be repeated before they can serve as cold or warm evidence.

## Consequences

- Platform runners do more packaging locally but avoid uploading and
  re-downloading complete tool bundles.
- Pull-request artifact storage drops from two complete three-platform copies
  to metadata, bootstrap binaries, the thin marketplace payload, assessment
  output, and the one Linux runtime required by assessment.
- Warm builds reuse Cargo compilation products without treating cached output
  as a release artifact or proof of a valid bundle.
- Release/tag behavior remains stricter than pull-request storage behavior: all
  targets are still packaged, published, downloaded again, and verified.
