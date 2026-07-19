# Unica Internal Package

This is maintainer reference material. Public skills describe 1C operations and
route through the one public MCP server `unica`; they do not expose package
infrastructure.

## Source and release boundaries

`third-party/tools.lock.json` is the dependency source of truth. The checked-in
`third-party/manifest.json` is a source placeholder only. CI builds the locked
tools for `darwin-arm64`, `linux-x64`, and `win-x64`, then
`package-unica-runtime.py` writes one deterministic archive per target. Each
archive contains `bin/<target>/` plus a target-specific generated tool manifest.

`package-unica-plugin.py` produces one target-neutral thin plugin. It copies
tracked skills/references/assets, three native bootstrap binaries, the portable
Git launcher, and a release-pinned `runtime-manifest.json`. It must not copy a
full runtime.

## Launch and verification

The packaged `.mcp.json` invokes `git` with a command-scoped `!` alias. Git's
shell executes `bootstrap/launch.sh`; the script maps the host to one bootstrap
without writing global Git configuration. This gives Git for Windows and POSIX
Git the same package contract without requiring Node.js.

The bootstrap:

1. validates source repository, source commit, release tag, target matrix, URLs,
   paths, and SHA-256 values in `runtime-manifest.json`;
2. serializes cache population with a per-version/target lock;
3. downloads over HTTPS into a UUID transaction directory;
4. verifies the archive while streaming, safely extracts only the declared
   files, verifies each file, and writes `.ready.json` last;
5. atomically renames the verified directory into
   `$CODEX_HOME/unica/runtimes/<version>/<target>`;
6. execs or supervises `unica` with inherited stdio.

The Rust bundled-tool resolver then reads the runtime's generated
`third-party/manifest.json` and re-checks each internal binary before execution.
Runtime stdout is JSON-RPC only.

## Frozen migration bridge

The immutable `v0.7.8` release is the only supported entry point for local,
duplicated, or otherwise legacy installations. Its published transition assets
retain the native transaction engine, rollback behavior, and exact historical
contracts used to produce a canonical marketplace installation.

The current `v0.8.0` source and thin package do not ship that engine, transition
scripts, legacy discovery models, or migration commands. Once `v0.7.8` has made
the installation canonical, later versions use the ordinary marketplace update
path.

## Publication

The source release workflow publishes and re-downloads the three runtime
archive/metadata pairs. It also emits the thin payload. Cross-repository
publication has two separate changes:

1. staging copies only `plugins/unica` and is merged before tagging;
2. promotion changes only the stable catalog after the staging commit has an
   existing immutable signed tag.

No published tag or release asset is moved or overwritten. Changed bytes require
a new plugin version.
