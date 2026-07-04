# 7. Представление развертывания

## Source Checkout

In the repository checkout, `.mcp.json` runs the Rust binary from the plugin
root through `cargo run --manifest-path ../../Cargo.toml --bin unica`. This
keeps local development possible without generated binary commits or shell
wrapper files.

## Generated Marketplace Package

The release pipeline builds target-specific bundled binaries, writes
`third-party/manifest.json`, and packages `plugins/unica`.

Packaged execution:

1. `.mcp.json` starts `./bin/<target>/unica` with `cwd` set to the plugin root.
2. The bundled `unica` binary starts as stdio MCP server.
3. Internal adapters resolve and verify their bundled tools through Rust before
   execution.

## Local Install

`scripts/dev/install-local-unica.sh` builds a local package, installs it as a
local Codex marketplace, validates native binaries, and can verify fresh Codex
prompt visibility.

## Runtime State

Volatile state defaults to `.build/unica` under the workspace root and can be
overridden by `UNICA_CACHE_DIR`.

Workspace-scoped internal services store runtime state under
`.build/unica/services/<service-key>/` or the equivalent `UNICA_CACHE_DIR`
location. The service record contains the localhost port, process id, token,
version, workspace root, source root, and access timestamps.

Packaged binaries locate the plugin root from their own executable path when
`UNICA_PLUGIN_ROOT` is not set, so hidden workspace services can locate internal
adapter assets even when the user workspace is outside the plugin directory.
