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
cancel — idempotent переход или чтение terminal winner. Один абсолютный budget
каждого get/result/cancel выводится один раз из исходного frontend cutoff. Для
result он дополнительно ограничен `waitMs + 125 мс`. Cutoff не переводится
обратно в duration при daemon admission и не перезапускается между connect,
handshake, request, response read и parse. Перед запросом daemon wait
уменьшается на уже израсходованное время и response margin; checkpoint после
parse имеет приоритет над поздним valid или malformed payload, закрывает
operation session и не публикует snapshot.

Working receipt — обычный structured-only `CallToolResult`: `ok`, `summary`,
`data.task` с `taskId`, полем состояния, `createdAtEpochMs`, `updatedAtEpochMs`, `ttlMs`,
`pollIntervalMs` и `next` с безопасным повтором `unica.task.result`. Пустые
optionals опускаются; ключей `job` и `work` нет. Completed возвращает тот же
canonical result, что direct предметный вызов. Invalid identity, bad wait,
unknown, expired, failed, cancelled и transport/projection failure имеют
различимые закрытые коды без daemon prose.

Допустимая форма Task исчерпывается матрицей: queued/working не имеют result и
failure; completed имеет только result; failed имеет только закрытый признак
failure; cancelled не имеет ни result, ни failure. Failure code/message не
проецируются. Любая другая форма в get и result возвращает
`task_projection_failed` до публикации содержимого.
