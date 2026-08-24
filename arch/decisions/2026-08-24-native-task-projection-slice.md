---
id: DEC.2026-08-24.NATIVE-TASK-PROJECTION-SLICE
status: active
governs: product
realized: crates/unica-coder/src/interfaces/mcp.rs::native_task_projection_contract_is_capability_gated_durable_and_replay_free
supersedes: []
superseded-by: null
establishes: [INV.WIRE.NATIVE-TASK-CAPABILITY, INV.WIRE.SDK-TRANSPORT, CTR.WIRE.NATIVE-TASK-PROJECTION, CTR.WIRE.DAEMON-INVOCATION-PROTOCOL]
changes: [CTR.WIRE.DAEMON-INVOCATION-PROTOCOL]
design: docs/design/2026-08-23-v0-13-execution-surface-design.md
---

# Скрытый V13 проецирует durable Invocation как SEP-2663 Task

**Решение.** Только явно injected `SurfaceRelease::V13`, запрос протокола
`2026-07-28` и capability `io.modelcontextprotocol/tasks` вместе разрешают
native Task projection. Версия или имя host сами по себе capability не создают;
профили `2025-11-25` и package-selected V12 Task не объявляют и не возвращают.

Обычный `tools/call` исполняет ту же daemon-owned Invocation один раз и отвечает
direct `CallToolResult` либо `CreateTaskResult`. `tasks/get` и idempotent
`tasks/cancel` читают/меняют только daemon durable store; `tasks/update` сначала
проверяет живую identity и затем возвращает закрытый
`task_input_not_supported`. Unknown, expired и noncanonical TaskId сохраняют
различимые закрытые ошибки. Polling и progress не запускают execution и после
`CreateTaskResult` progress не продолжается.

Daemon protocol v3 переносит сохранённые epoch timestamps, TTL и poll interval;
его строка ABI identity `unica-daemon-jsonl-3` входит в `CoreIdentity`, поэтому
consumer старой версии не может разделить daemon state с новым protocol;
ISO-8601 и все типы `rmcp` появляются только в `interfaces/`. Direct result и
terminal Task используют один renderer, поэтому отличаются только protocol
Task wrapper. Публичная поверхность и package V12 не меняются до Task 22.

**Почему.** Capability-gated projection добавляет native lifecycle без второго
источника состояния и без промежуточной публикации v0.13.
**Цена.** До cutover путь доступен только injected-профилю и проверочным seam.
