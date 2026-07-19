# Issue 123: Public Git Marketplace and Thin Plugin Design

**Date:** 2026-07-19  
**Status:** Approved for implementation planning  
**Target release:** `v0.7.0`  
**Source issue:** `IngvarConsulting/unica#123`

## 1. Purpose

Move consumer delivery from a locally copied release marketplace to the public
Git marketplace `IngvarConsulting/unica-marketplace`. The installed identity is
always `unica@unica`, and the only public MCP server remains `unica`.

The change fixes the delivery model that caused issue 90. Renaming the current
`unica-local` alias would only hide one collision while preserving manual
marketplace copies, manual plugin-cache writes, and snapshots that Codex cannot
upgrade through its native marketplace commands.

## 2. Resolved Constraints and Contradictions

### 2.1 Git is an explicit prerequisite

The command

```text
codex plugin marketplace add IngvarConsulting/unica-marketplace --ref main
```

uses the external `git` executable in the current Codex implementation. A Git
marketplace cannot support a machine without Git while keeping this installation
contract. The installer and migration helpers must therefore fail in preflight,
before mutations, when Git or Git shell aliases are unavailable.

This is the only external runtime prerequisite introduced by the consumer
delivery path. Node.js, Python, PowerShell Core, curl, wget, and a system archive
utility are not prerequisites for starting Unica.

### 2.2 A Node bootstrap is not acceptable

A single JavaScript bootstrap would make `node` an undocumented prerequisite
and would fail the new-user acceptance criterion on machines without Node.js.
The plugin instead contains three small native bootstrap binaries, one for each
supported target. These are package utilities, not the Unica platform runtime.

### 2.3 One `.mcp.json` needs a platform-neutral entry point

The current plugin MCP configuration accepts one stdio `command`; it has no
documented per-OS command selector. The stable entry point is therefore `git`,
which is already required to materialize the marketplace. A one-shot `!` alias
uses Git's own shell to run the checked-in POSIX launcher on every supported
platform, including Git for Windows.

### 2.4 Publication requires two marketplace commits

A marketplace catalog cannot safely point to a tag that does not exist yet. A
single PR that both adds a package and points stable at a future tag creates a
failure window. Consumer publication therefore has two phases:

1. merge the new package while the stable catalog still points to the previous
   version, then create the immutable signed tag;
2. merge a promotion PR that changes the stable catalog to the existing tag.

The promotion merge is the atomic consumer-publication point.

## 3. Goals

- Publish one public marketplace named `unica`.
- Install one plugin selector, `unica@unica`.
- Preserve one public stdio MCP server named `unica`.
- Download only the full runtime for the current host target.
- Pin every downloaded byte to plugin version, source commit, release tag, and
  SHA-256.
- Make interrupted and concurrent bootstrap attempts safe.
- Migrate known legacy `unica-local` layouts transactionally on POSIX and
  Windows.
- Use native `codex plugin` commands for marketplace, plugin, cache, and config
  ownership.
- Publish only after source, release assets, marketplace package, fresh install,
  upgrade, and runtime behavior have been verified.

## 4. Non-goals

- Background or automatic updates.
- A remote HTTPS Unica MCP service.
- Publication in the official OpenAI Plugins Directory.
- Keeping `unica-local` as a consumer fallback.
- Shipping full platform runtimes in the Git marketplace.
- Making assessment Pages a prerequisite for consumer publication.

## 5. Repository Responsibilities

### 5.1 `IngvarConsulting/unica`

This repository remains authoritative for:

- Rust and supporting source code;
- source tests and contract tests;
- native `unica` and bundled tool builds;
- native bootstrap source and target builds;
- runtime archive generation;
- runtime manifest generation;
- immutable Git tag and GitHub Release assets;
- migration and update helper source;
- cross-repository publication automation.

Live code, tests, `Cargo.toml`, plugin metadata, `.mcp.json`, and
`third-party/tools.lock.json` override historical plans when they disagree.

### 5.2 `IngvarConsulting/unica-marketplace`

This new public repository is authoritative for consumer package snapshots:

```text
.agents/plugins/marketplace.json
plugins/unica/
  .codex-plugin/plugin.json
  .mcp.json
  skills/
  assets/
  bootstrap/
    launch.sh
    bin/
      darwin-arm64/unica-bootstrap
      linux-x64/unica-bootstrap
      win-x64/unica-bootstrap.exe
  runtime-manifest.json
README.md
MIGRATION.md
.github/workflows/verify.yml
```

The repository contains no full Unica runtime and no `unica-local` identity.

## 6. Stable Identities and Version Contract

| Entity | Value |
|---|---|
| Marketplace | `unica` |
| Plugin | `unica` |
| Plugin selector | `unica@unica` |
| MCP server | `unica` |
| Stable catalog branch | `main` |
| Initial target version | `0.7.0` |

The following values must be equal for a publication:

```text
Git tag
= Cargo workspace and unica-coder version
= plugin.json version
= tools.lock Unica version
= native unica --version
= runtime-manifest pluginVersion
= marketplace package version and immutable ref
```

Published tags and assets are never overwritten. Any changed published byte
requires a new version.

## 7. Marketplace Catalog Contract

The stable catalog is committed on `main`, but the plugin entry uses a
Git-backed `git-subdir` source pinned to an immutable marketplace tag or commit.
It never uses `source: local`, `latest`, or a version range.

Conceptually:

```json
{
  "name": "unica",
  "plugins": [
    {
      "name": "unica",
      "source": {
        "source": "git-subdir",
        "url": "https://github.com/IngvarConsulting/unica-marketplace.git",
        "path": "./plugins/unica",
        "ref": "v0.7.0"
      },
      "policy": {
        "installation": "AVAILABLE",
        "authentication": "ON_INSTALL"
      },
      "category": "Coding"
    }
  ]
}
```

## 8. Platform-neutral MCP Launch

### 8.1 `.mcp.json`

The package exposes one command shape for all operating systems:

```json
{
  "mcpServers": {
    "unica": {
      "command": "git",
      "args": [
        "-c",
        "alias.unica-bootstrap=!f() { exec sh \"$PWD/${GIT_PREFIX:-}bootstrap/launch.sh\"; }; f",
        "unica-bootstrap"
      ],
      "cwd": "."
    }
  }
}
```

The alias is command-scoped through `git -c`; it does not edit global, system,
or repository Git configuration. Git executes `!` aliases through its shell.
`GIT_PREFIX` keeps the launcher address correct when a development plugin root
is inside a Git checkout; installed cache copies normally use an empty prefix.

All launcher and bootstrap diagnostics go to stderr. Stdout remains exclusively
the MCP transport.

### 8.2 `bootstrap/launch.sh`

The launcher is LF-terminated POSIX shell with no external command dependency
other than shell built-ins and `uname`, supplied by the Git environment. It
maps:

- `Darwin` plus `arm64` or `aarch64` to `darwin-arm64`;
- `Linux` plus `x86_64` or `amd64` to `linux-x64`;
- `MINGW*`, `MSYS*`, or `CYGWIN*` plus `x86_64` or `amd64` to `win-x64`.

It rejects every other combination with exit code 78 and an actionable error.
It then replaces itself with the matching native bootstrap binary.

### 8.3 Native bootstrap binaries

A new Rust binary package, `unica-bootstrap`, is built for:

- `aarch64-apple-darwin`;
- `x86_64-unknown-linux-gnu`;
- `x86_64-pc-windows-msvc`.

The binary owns HTTPS download, SHA-256 verification, safe tar extraction,
locking, cache publication, and runtime process supervision. It does not expose
public Unica operations and is not a second MCP server.

## 9. Runtime Manifest

`runtime-manifest.json` is deterministic and contains no generation timestamp.
Its schema records:

- schema version;
- plugin version;
- source repository and exact commit;
- release repository and exact tag;
- one record per supported target;
- immutable asset name and URL;
- archive media type and SHA-256;
- expected relative file paths and SHA-256 values;
- executable bits where relevant;
- target-specific `unica` entry point;
- manifest digest used by the ready marker.

The manifest and `plugin.json` versions must match before any network access.
URLs must use HTTPS and the expected `IngvarConsulting/unica` release origin.

All target runtimes use a `.tar.gz` container, including Windows, so one audited
extractor handles every platform. Archives contain only the target runtime, not
marketplace metadata or other targets.

## 10. Runtime Cache and Bootstrap Transaction

The default cache root is:

```text
${CODEX_HOME:-<platform default>}/unica/runtimes/<version>/<target>/
```

Tests may override it with `UNICA_RUNTIME_CACHE_DIR`. The cache identity contains
no marketplace alias and no `unica-local` segment.

Bootstrap performs these steps:

1. Parse and validate the embedded runtime manifest.
2. Detect the current target and select exactly one manifest record.
3. Open a version-target lock file and acquire an OS-backed exclusive lock.
   Locks are released automatically on process exit or crash.
4. Recheck the ready marker after acquiring the lock.
5. Create unique download and extraction paths on the same filesystem as the
   final runtime directory.
6. Download the exact asset without following redirects to an unapproved final
   scheme.
7. Verify archive SHA-256 before extraction.
8. Reject absolute paths, parent traversal, links, devices, and unsupported tar
   entries during extraction.
9. Verify every expected file SHA-256 and reject missing or unexpected runtime
   files.
10. Write a ready marker containing version, target, and manifest digest.
11. Atomically rename the completed directory into its versioned location.
12. Remove only transaction-owned temporary paths.
13. Start the target `unica` binary with inherited stdin, stdout, and stderr.

If another process wins publication, the loser discards only its temporary
directory and uses the verified winner. A partial download or extraction never
receives a ready marker and is never treated as runnable.

On Unix, bootstrap replaces itself with the runtime process. On Windows, it
supervises the runtime in a Job Object, propagates termination, and exits with
the runtime exit code so a killed bootstrap cannot leave an orphaned MCP server.

## 11. Installation and Explicit Update

### 11.1 New installation

Prerequisites are a compatible Codex CLI and standard Git.

```bash
codex plugin marketplace add IngvarConsulting/unica-marketplace --ref main
codex plugin add unica@unica
```

The user then opens a new Codex task. The first MCP start downloads and verifies
the pinned runtime for the current platform.

### 11.2 Explicit update

```bash
codex plugin marketplace upgrade unica
codex plugin remove unica@unica
codex plugin add unica@unica
```

The user opens a new task after verification. Documentation must not claim that
marketplace upgrade automatically replaces an installed plugin-cache snapshot.

The update helper wraps only these native commands and verification. It never
copies a marketplace or plugin cache and never edits `config.toml` normally.

## 12. Legacy Migration

Both `scripts/install-unica.sh` and `scripts/install-unica.ps1` become migration
and update shims. Existing release URLs remain available for a limited migration
window, but the scripts no longer download a local marketplace archive.

### 12.1 Preflight

Before mutation, each helper:

1. verifies Codex CLI capabilities using actual `--json` commands rather than a
   guessed version comparison;
2. verifies `git` and a one-shot `!` shell alias;
3. reads marketplace and plugin state through Codex JSON commands;
4. detects known legacy registrations, duplicated selectors, local sources, and
   orphaned config/cache paths;
5. rejects a marketplace named `unica` that belongs to an unknown source;
6. identifies the exact config, marketplace, and cache paths that may change;
7. creates a timestamped backup and diagnostic log containing no secrets.

If the canonical Git source is already installed, the helper enters idempotent
update mode.

### 12.2 Transaction

The migration transaction:

1. removes legacy plugin registrations through Codex CLI;
2. removes legacy marketplace registrations by their actual manifest names;
3. preserves legacy directories and the backup until all verification succeeds;
4. adds `IngvarConsulting/unica-marketplace --ref main`;
5. installs `unica@unica` with JSON output;
6. verifies installed version and source identity;
7. asks the native bootstrap to verify or install the pinned runtime;
8. runs MCP `initialize` and `tools/list` verification through the bootstrap
   verification subcommand;
9. requires `unica.project.status`, `unica.standards.search`, and
   `unica.standards.explain` in the tool list;
10. verifies prompt-visible skills through a fresh Codex prompt-input proof;
11. deletes only the exact legacy directories and caches identified in preflight.

An active task may retain its old MCP process. Success output must tell the user
to create a new task or restart the client.

### 12.3 Rollback

Any failure after the first mutation triggers rollback:

1. remove the partial new plugin and marketplace through Codex CLI;
2. restore the backed-up config and exact legacy paths;
3. restore legacy marketplace registration through Codex CLI when possible;
4. verify that the legacy installation is discoverable again;
5. retain backup and redacted diagnostics;
6. exit nonzero with the next safe command.

Rollback itself is failure-injected and tested at every mutating stage.

## 13. Release and Cross-repository Publication

### Phase A: Runtime release in `unica`

1. Enforce the unified version contract.
2. Build native bootstrap binaries and full runtime archives for all targets.
3. Run source, package, bootstrap, MCP, and runtime smoke tests.
4. Create the immutable signed `v0.7.0` source tag.
5. Publish runtime assets and checksums to the GitHub Release.
6. Download the published assets again and verify their contents and hashes.
7. Generate the thin marketplace package and deterministic runtime manifest
   from the verified published assets.

Assessment Pages may run independently and may report their own failure, but
they do not block or gate runtime asset publication.

### Phase B: Stage package in `unica-marketplace`

1. Automation opens a staging PR containing the new thin package while the
   stable catalog still references the previous release.
2. Marketplace CI verifies metadata, forbidden terms, bootstrap hashes, package
   size, fresh installation, runtime bootstrap, and upgrade on macOS, Linux, and
   Windows.
3. Merge the staging PR.
4. Create an immutable signed marketplace tag at the staging merge commit.

### Phase C: Promote stable catalog

1. Automation opens a promotion PR changing only the stable catalog ref.
2. CI verifies the referenced tag exists and matches the expected package and
   runtime manifest.
3. Merge the promotion PR. This is consumer publication.
4. In clean isolated homes on all three platforms, add marketplace `main`, add
   `unica@unica`, start MCP, and verify the published runtime and tool list.

Failures before the promotion merge leave the previous stable pointer intact.
Rollback after promotion is a new promotion commit pointing to the last known
good immutable marketplace tag; published tags are not moved or deleted.

## 14. Development-only Installation

Contributor installation remains separate in
`scripts/dev/install-local-unica.sh`. If a marketplace name is needed, it is
`unica-dev`, never `unica-local`. Development artifacts are excluded from release
assets, consumer documentation, and consumer verification contracts.

## 15. Documentation

Consumer documentation covers:

- Git and Codex prerequisites;
- new installation;
- first-start runtime download;
- explicit update;
- legacy migration;
- rollback and backup location;
- uninstall;
- optional removal of inactive versioned runtime caches;
- the requirement to open a new task after install, update, or migration.

Uninstall uses Codex CLI to remove the plugin and marketplace. Runtime-cache
deletion is a separate explicit helper action so uninstall does not silently
delete diagnostic or rollback evidence.

## 16. Verification Strategy

### 16.1 Unit and contract tests

- version-contract equality;
- deterministic manifest generation;
- target detection, including Git for Windows `uname` variants;
- manifest origin and checksum validation;
- cache and ready-marker state transitions;
- archive traversal, link, device, missing-file, extra-file, and corruption
  rejection;
- lock contention and crash recovery;
- exact exit-code propagation;
- no consumer `unica-local` strings;
- no `source: local` in generated consumer metadata;
- no manual consumer cache/config writes.

### 16.2 Integration tests

- `.mcp.json` executes through a one-shot Git alias outside a Git checkout;
- the same command works from a development plugin inside a checkout;
- Windows execution reaches `unica-bootstrap.exe` without Node, WSL, or a
  separately installed shell;
- two simultaneous first starts publish one valid runtime;
- interrupted download and extraction leave no ready runtime;
- MCP `initialize` and `tools/list` work through bootstrap;
- migration is idempotent;
- every mutating migration stage restores the legacy installation on failure.

### 16.3 Published-system proof

Completion requires evidence from:

- source commit and signed source tag;
- successful source and marketplace workflows;
- `gh release view` and re-downloaded runtime assets;
- unpacked published thin package;
- immutable marketplace tag;
- stable catalog ref;
- isolated fresh installs on all targets;
- isolated upgrade from the previous stable release;
- a new Codex task using the published package and downloaded runtime.

Local build-tree success alone is not release proof.

## 17. Acceptance Criteria

- `IngvarConsulting/unica-marketplace` is public and its stable catalog points to
  an existing immutable package tag.
- A machine with compatible Codex and standard Git installs with only the two
  documented `codex plugin` commands.
- Node.js and other scripting runtimes are not required.
- Consumer paths, docs, scripts, release names, and generated metadata contain
  no `unica-local`.
- Exactly one `unica@unica` registration and one `unica` MCP server are active.
- Only the current target's full runtime is downloaded.
- Archive and individual files are verified before publication into cache.
- Interrupted and concurrent bootstrap attempts cannot publish corrupt state.
- Fresh install and explicit update pass on all supported targets.
- POSIX and Windows legacy migrations are idempotent and rollback-safe.
- Published `tools/list` includes `unica.project.status`,
  `unica.standards.search`, and `unica.standards.explain`.
- Install, update, migration, rollback, and uninstall documentation is complete.

