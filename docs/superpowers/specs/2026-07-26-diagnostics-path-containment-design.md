# Diagnostics path containment design

## Goal

`unica.code.diagnostics` must accept `path` only for file diagnostics and must
reject malformed or out-of-root file paths before it invokes `bsl-analyzer`.

## Scope

The contract layer permits `path` only when the resolved diagnostics mode is
`file`; the default `analyze` mode and the explicit `analyze`, `status`,
`catalog`, and `workspace` modes reject it. The adapter additionally requires
every present `path` value to be a string and enforces file-path containment
within the resolved `sourceDir`.

The change does not alter source-root selection or graph requests.

## Design

`tool_contracts.rs` resolves the omitted mode to `analyze`, rejects `path` for
every non-file mode, and continues to require `path` for non-dry-run file
requests.

The BSL MCP adapter resolves `sourceDir` through the existing
`resolve_source_root` path. Before selecting or invoking a runner, it rejects a
present non-string `path`. Before building the diagnostics MCP payload, it
resolves a string `path` relative to that root, normalizes the identity through
existing ancestors (therefore following symlinks), and requires the result to
start with the normalized source-root identity.

On failure, the adapter returns one stable, actionable error prefixed
`invalid_diagnostics_path:`. The diagnostic request is not sent to
`bsl-analyzer`.

## Rejected inputs

- a `path` supplied for a diagnostics mode other than `file`;
- a present `path` whose JSON value is not a string;
- a relative path escaping the source root through `..`;
- an absolute path outside that root;
- a path that enters an in-root symlink whose target is outside that root.

## Tests

Contract tests preserve the omitted-mode `analyze` default and cover explicit
non-file modes. Adapter tests use the recording BSL MCP runner. Each malformed
or out-of-root adapter case must assert the stable error prefix and that the
runner received no command. A valid relative path must still produce the
existing typed `diagnostics` MCP payload unchanged.
