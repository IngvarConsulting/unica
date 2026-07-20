# 7. Представление развертывания

## Source checkout

The tracked plugin is a development source tree. Its `.mcp.json` runs the Rust
workspace with Cargo; generated binaries are not committed. The local debug
script builds one host target and registers a distinct `unica-dev` marketplace.

## Source release

`IngvarConsulting/unica` publishes deterministic full runtime archives and JSON
metadata for `darwin-arm64`, `linux-x64`, and `win-x64`. Each archive contains
only one target. Published bytes are downloaded again and verified before the
marketplace publication workflow can succeed.

## Public marketplace

`IngvarConsulting/unica-marketplace` stores a thin plugin at `plugins/unica`.
The stable `.agents/plugins/marketplace.json` entry uses `git-subdir` and an
immutable marketplace tag. Staging changes plugin files only; promotion changes
the catalog only after the staging merge commit is tagged.

## Consumer host

Codex stores the thin plugin in its managed plugin cache. `.mcp.json` launches
through standard Git, so the same command works with POSIX Git and Git for
Windows. The selected native bootstrap downloads the current target runtime to
`$CODEX_HOME/unica/runtimes/<version>/<target>`, validates it, and starts the
single public MCP process.

Git and Codex CLI are required. Node.js, Python, HTTP clients, JSON tools, and
archive utilities are not part of the consumer deployment.

## State and rollback

Volatile workspace/cache state stays under `.build/unica` or `UNICA_CACHE_DIR`.
Branched-development task/operation/lock/recovery state instead stays under the
OS per-user application-state directory or `UNICA_STATE_DIR`; it survives cache
deletion and has explicit schema migration and durable-write semantics.
A small owner-only coordination root resolved from the OS user profile is not
overridden; it locates the registered durable root and unresolved tasks across
process restarts and override changes.

Large task IBs, XML, sandboxes, checkpoints, artifacts, and raw logs live below
the configured task work root in one marker-owned UUID instance. Successful or
safely abandoned cleanup quarantines and deletes only that instance; compact
redacted state/archive remain under the durable state root.

Downloaded runtime state is version/target scoped and guarded by a ready marker.
Migration backups live under `$CODEX_HOME/unica/migration-backups`; failed
migration restores configuration atomically and reverses successful Codex
mutations.
