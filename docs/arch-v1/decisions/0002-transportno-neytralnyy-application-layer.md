# ADR-0002: Транспортно-нейтральный application layer

- Статус: `accepted`
- Дата: `2026-05-03`

## Контекст

MCP transport is only one way to invoke Unica workflows. If business
orchestration depends directly on JSON-RPC payloads, future CLI, tests, or
internal automation will duplicate behavior or bypass cache/event handling.

## Решение

Keep the application layer transport-neutral.

1. `UnicaApplication` exposes tool registry and tool call behavior.
2. `interfaces::mcp` maps MCP requests to application calls.
3. Domain events and cache reporting are produced in application flow, not in
   MCP transport.
4. Infrastructure adapters do not render MCP responses.

## Неграницы

1. MCP-specific DTO mapping remains allowed in the MCP interface layer.
2. The application layer may know public tool names as stable operation ids.
3. This ADR does not require adding a separate CLI.

## Последствия

1. Adapter and domain tests can run without spawning stdio MCP.
2. MCP response shape can evolve separately from business orchestration.
3. New tool handlers must enter through application dispatch.

## План реализации

1. Keep `interfaces::mcp` limited to protocol handling.
2. Keep tool dispatch in `application`.
3. Add unit tests for application behavior and separate MCP smoke tests.

## Верификация

- [x] ADR states responsibility split between MCP and application.
- [x] ADR keeps cache/event behavior out of transport-only code.
