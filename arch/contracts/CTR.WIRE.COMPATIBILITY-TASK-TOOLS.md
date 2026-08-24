---
id: CTR.WIRE.COMPATIBILITY-TASK-TOOLS
status: active
governs: product
decision: DEC.2026-08-24.COMPATIBILITY-TASK-TOOLS-SLICE
check: crates/unica-coder/src/interfaces/mcp.rs::v13_compatibility_task_tools_are_profile_gated_durable_and_replay_free
scope: [wire]
version: 1
producer: crates/unica-coder/src/interfaces/mcp.rs
consumers: [host]
---

# Три compatibility-инструмента durable Task

`unica.task.get` и `unica.task.cancel` требуют один canonical opaque `taskId`.
`unica.task.result` требует `taskId` и принимает integer `waitMs` в диапазоне
0..=7000; по умолчанию 7000. Get — immediate probe, result — bounded wait,
cancel — idempotent переход или чтение terminal winner.

Working receipt — обычный structured-only `CallToolResult`: `ok`, `summary`,
`data.task` с `taskId`, полем состояния, `createdAtEpochMs`, `updatedAtEpochMs`, `ttlMs`,
`pollIntervalMs` и `next` с безопасным повтором `unica.task.result`. Пустые
optionals опускаются; ключей `job` и `work` нет. Completed возвращает тот же
canonical result, что direct предметный вызов. Invalid identity, bad wait,
unknown, expired, failed, cancelled и transport/projection failure имеют
различимые закрытые коды без daemon prose.
