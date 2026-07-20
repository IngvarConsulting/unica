# 4. Стратегия решения

## Strategy

Use a pragmatic DDD split:

- domain: workspace identity, cache impact, domain events;
- application: tool registry, use case dispatch, orchestration;
- infrastructure: internal adapters and filesystem state;
- interfaces: MCP JSON-RPC transport.

Branched development adds a separate bounded context with pure state/delta/lock
rules, transport-neutral use cases, typed Designer and secret ports, durable
operation records, and platform-specific execution behind the existing
infrastructure facade.

## Key Decisions

1. Hide all engines behind one MCP server.
2. Keep application logic transport-neutral.
3. Emit domain events for mutating operations.
4. Let cache invalidation happen inside `unica`.
5. Keep operation backend command semantics inside native Rust MCP handlers.
6. Treat repository effects as journaled, capability-gated operations whose
   success is defined by observed postconditions, not process exit alone.
7. Keep dangerous operational state separate from volatile cache state.

## Migration Shape

New operation backend behavior is ported into Rust first, with donor scripts
retained only as parity fixtures when they are needed as a source model.
