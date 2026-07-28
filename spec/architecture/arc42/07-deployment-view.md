# 7. Представление развертывания

Развертывание Unica проходит четыре контура: рабочее дерево разработчика,
релиз исходного репозитория, публичный маркетплейс и машина потребителя. Ниже
описан каждый контур и то, что его удерживает.

## Source checkout

The tracked plugin is a development source tree. Its `.mcp.json` runs the Rust
workspace with Cargo, and generated binaries are never committed
(INV-PKG-01). The local debug packaging script builds one host target and
rewrites `.mcp.json` to launch that target's `unica` binary directly, without
the bootstrap payload the published package carries, so a development install
never takes the published launch path (INV-PKG-07). The catalog name separates
the two only on Codex, where the generated catalog is renamed (`unica-dev` by
default); the Claude catalog is derived from the packaged manifest and keeps the
published name `unica`.

## Source release

`IngvarConsulting/unica` publishes deterministic full runtime archives and JSON
metadata for `darwin-arm64`, `linux-x64`, and `win-x64`. Each archive contains
exactly one target. Published bytes are downloaded again and verified before the
marketplace publication workflow can succeed (INV-CI-04), and publication runs
only from a tag (INV-CI-05).

## Public marketplace

`IngvarConsulting/unica-marketplace` stores a thin plugin at `plugins/unica`:
the package carries native bootstrap binaries and plugin metadata, not a full
runtime (INV-PKG-02). Two catalogs describe the same package, one per host, and
both resolve it through `git-subdir` at an immutable marketplace tag:

| Host | Catalog | Manifest directory |
| --- | --- | --- |
| Codex | `.agents/plugins/marketplace.json` | `.codex-plugin/plugin.json` |
| Claude Code | `.claude-plugin/marketplace.json` | `.claude-plugin/plugin.json` |

Both manifests carry the same version (INV-PKG-05) and stay inside the key set
that the oldest supported client accepts (INV-PKG-06); for Claude Code that
floor is 2.1.69. Staging changes plugin files only; promotion changes the
catalogs only after the staging merge commit is tagged.

## Consumer host

Both hosts store the thin plugin in their own managed plugin cache and launch it
the same way: `.mcp.json` runs standard Git with a command-scoped alias, which
starts the portable bootstrap selector. The same command therefore works with
POSIX Git and with Git for Windows. The selected native bootstrap downloads the
runtime for the current target, verifies it against the recorded checksum,
installs it atomically, and only then starts the single public MCP process
(INV-PKG-03, INV-MCP-01).

Prerequisites are Git plus one host: Codex CLI, or Claude Code 2.1.69 or newer.
Node.js, Python, HTTP clients, JSON tools, and archive utilities are not part of
the consumer deployment.

The runtime cache root is resolved deterministically, first match wins
(INV-CACHE-07):

1. `UNICA_RUNTIME_CACHE_DIR` as given, which the package sets for hosts that
   provide their own per-plugin data directory; a value that still contains an
   unexpanded `${` token is discarded, because the host did not substitute it;
2. `<CLAUDE_PLUGIN_DATA>/runtimes`;
3. `<CODEX_HOME>/unica/runtimes`; without `CODEX_HOME` the home directory takes
   over with a `.codex` segment, so the path is
   `<HOME or USERPROFILE>/.codex/unica/runtimes`.

When none of these variables is set, bootstrap fails with a typed error instead
of guessing a location.

Within that root, runtime state is scoped by version and target and guarded by a
ready marker, so a partially downloaded runtime is never handed to the host.

## State and rollback

Workspace state stays under `.build/unica` or the root named by
`UNICA_CACHE_DIR` (INV-CACHE-03). It is volatile: deleting it costs a rebuild,
never a correctness problem. Runtime state is version- and target-scoped, so
installing a new version does not mutate the previous one and rollback is a
matter of selecting the earlier version.

Current packages carry no legacy migration engine: installations older than the
thin-runtime contract move forward through the immutable `v0.7.8` bridge release
rather than through an in-package migration path (ADR-0008).
