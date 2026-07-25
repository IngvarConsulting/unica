# Diagnostics path containment design

## Goal

`unica.code.diagnostics` must reject a file path that does not belong to its
resolved `sourceDir` before it invokes `bsl-analyzer`.

## Scope

The rule applies to diagnostics modes that accept `path` (`file` and any
future typed diagnostics mode that forwards it). It does not change source-root
selection, graph requests, or the full-source `analyze` mode, which already
rejects `path` in #198.

## Design

The BSL MCP adapter resolves `sourceDir` through the existing
`resolve_source_root` path. Before building the diagnostics MCP payload, it
will resolve the supplied `path` relative to that root, normalize the identity
through existing ancestors (therefore following symlinks), and require the
result to start with the normalized source-root identity.

On failure, the adapter returns one stable, actionable error prefixed
`invalid_diagnostics_path:`. The diagnostic request is not sent to
`bsl-analyzer`.

## Rejected inputs

- a relative path escaping the source root through `..`;
- an absolute path outside that root;
- a path that enters an in-root symlink whose target is outside that root.

## Tests

Adapter tests will use the recording BSL MCP runner. Each rejected case must
assert the stable error prefix and that the runner received no command. A
valid relative path must still produce the existing typed `diagnostics` MCP
payload unchanged.
