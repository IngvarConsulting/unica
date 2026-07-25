# ADR-0013: MCP transport is owned by the official Rust SDK

- Status: accepted
- Date: 2026-07-26

## Context

`interfaces/mcp.rs` reimplemented the MCP stdio transport by hand: a JSON-RPC
read loop over `std::thread`, a bespoke worker pool, a cancellation registry,
and a hard-coded `protocolVersion = "2024-11-05"` that was returned without
negotiating with the client. The Model Context Protocol specification keeps
moving (structured output, elicitation, progress notifications, newer protocol
revisions), and every revision widened the silent gap between the hand-rolled
transport and the upstream contract. The official Rust SDK (`rmcp`) is
maintained, tested against the specification, and already implements the
behaviors we duplicated: per-request task spawning, `notifications/cancelled`
handling, `ping`, protocol version negotiation, and EOF drain.

The cost of adopting the SDK is a `tokio` runtime inside a previously fully
synchronous binary, and the discipline to keep our data-driven tool contract
out of the SDK's macro layer. Issue #219 records the decision driver: stop
hand-maintaining transport code; binary size is explicitly not a constraint.

## Decision

The public `unica` stdio MCP server runs on `rmcp` (official Rust SDK), with
`default-features = false, features = ["server", "transport-io"]`.

1. `interfaces/mcp.rs` implements `rmcp::ServerHandler` directly. The SDK's
   `#[tool]`/`#[tool_router]` macros are not used: tool names, descriptions,
   and input schemas remain data-driven by `application/operation_descriptors.rs`
   and `application/tool_contracts.rs` (ADR-0001 contract surface).
2. `rmcp` types do not leak past `interfaces/mcp.rs`. The application layer
   keeps its transport-neutral API (ADR-0002): `UnicaApplication::tools()` and
   `call_tool_cancellable` with the domain `CancellationToken`.
3. Tool execution runs in `tokio::task::spawn_blocking`; the SDK's per-request
   cancellation token is bridged to the domain token. Concurrent `tools/call`
   admission stays bounded at 32; excess calls fail with JSON-RPC `-32603`
   containing `overloaded`.
4. Tool execution failures keep their current wire shape (JSON-RPC error
   `-32000`). Moving them to `CallToolResult.isError` is a separate,
   deliberate contract change tracked outside this migration.
5. On transport shutdown the server cancels still-running domain operations and
   waits a bounded grace for them, preserving child-process cleanup guarantees.

## Contract deltas accepted with the SDK

These are behavior changes relative to the hand-rolled loop, accepted because
they match the MCP specification:

1. `protocolVersion` is negotiated with the client instead of being pinned to
   `2024-11-05`; the server can speak newer protocol revisions.
2. A strict handshake: the first request must be a well-formed `initialize`
   (`ping` is allowed before it). Requests sent without a handshake, tolerated
   by the old loop, are no longer served.
3. A request cancelled via `notifications/cancelled` gets no response at all
   (the specification says the response SHOULD NOT be sent); the old loop
   answered with `-32800`.
4. The 8 MiB public input line bound is delegated to the SDK, which currently
   does not bound line length. The stdio peer is the host process in the same
   trust domain, so this was defense-in-depth, not a security boundary. The
   internal workspace-service protocol keeps its own 8 MiB bound and worker
   limits (unchanged).
5. EOF drain timing follows the SDK (up to 5 s natural drain, then our bounded
   cancellation grace) instead of the bespoke 250 ms/2 s schedule.

## Relation to ADR-0008

"Thin" in ADR-0008 constrains the marketplace package and its acquisition
path, not the runtime's dependency graph. The runtime binary embedding an
async runtime does not violate that decision; ADR-0008 is amended with one
clarifying sentence to prevent the opposite reading.

## Consequences

1. ~1300 lines of transport code are deleted and stop being maintained here.
2. `tokio` becomes a workspace dependency of `unica-coder`; the binary is
   larger. Accepted (issue #219).
3. Transport-level unit tests move from feeding raw JSON strings into a
   hand-rolled dispatcher to driving a real SDK server over an in-memory
   duplex transport.
4. A second transport (streamable HTTP) or `resources`/`prompts` capabilities
   become feature flags away instead of a second hand-rolled implementation.
5. Protocol-revision upgrades become SDK version bumps.

## Verification

- [ ] `tools/list` over the SDK returns the same tools with the same
  data-driven schemas as before the migration.
- [ ] `ping` stays responsive while a `tools/call` runs; cancellation reaches
  the domain token; admission stays bounded at 32.
- [ ] Both hosts (Codex and Claude Code) complete `initialize`/`tools/list`
  against the SDK server.
- [ ] `spec/acceptance/unica-mcp-validation.md` describes the SDK-era wire
  contract (handshake, cancellation, EOF, line bound).
