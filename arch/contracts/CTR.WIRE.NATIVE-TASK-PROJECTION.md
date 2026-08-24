---
id: CTR.WIRE.NATIVE-TASK-PROJECTION
status: active
governs: product
decision: DEC.2026-08-24.NATIVE-TASK-PROJECTION-SLICE
check: crates/unica-coder/src/interfaces/mcp.rs::native_task_projection_contract_is_capability_gated_durable_and_replay_free
scope: [wire]
version: 1
producer: crates/unica-coder/src/interfaces/task_projection.rs
consumers: [host]
---

# SEP-2663 projection скрытого V13

`tools/call` может вернуть `CreateTaskResult`; `tasks/get` возвращает
`DetailedTask`, `tasks/cancel` — idempotent complete ack. `tasks/update` для
существующей неистёкшей Task возвращает `task_input_not_supported`, а unknown,
expired и noncanonical identity не маскируются. Task timestamps — стабильный
ISO-8601 projection durable epoch values; TTL и poll interval также приходят из
store. В `completed.result` лежит байт-в-байт тот же `CallToolResult`, что в
direct-ответе, включая content order, `isError`, `_meta`, `resultType` и
`structuredContent`.
