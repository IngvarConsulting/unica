# ADR-0014: rmcp owns the public MCP transport

- Status: accepted
- Date: 2026-07-26

## Context

`interfaces::mcp` currently maintains a bespoke stdio JSON-RPC loop, protocol
version constant, worker pool, cancellation routing and response encoding.
The public contract remains one server, `unica`, with data-driven `unica.*`
tool names and schemas defined by application contracts.

## Decision

1. `rmcp` owns stdio transport, protocol negotiation and MCP server plumbing.
2. `interfaces::mcp` is the only layer that imports `rmcp` or `tokio`.
3. The interface builds the SDK tool registry from existing application
   descriptors; `#[tool]`-derived schemas are not used.
4. Application dispatch, data-driven schemas, `unica.*` names, redaction,
   bounded concurrency and cancellation remain compatibility requirements.
5. `schemars` is limited to SDK boundary DTOs and does not replace application
   tool-contract schemas.

## Consequences

- ADR-0008 permits the supported async runtime dependencies.
- Manual JSON-RPC transport is removed only after equivalent cancellation,
  concurrency, ping and host-compatibility tests pass.
- A later increment may add SDK transports without a new public server name.
