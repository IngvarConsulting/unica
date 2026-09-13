---
id: DEC.2026-08-23.USER-CORE-DAEMON-SLICE
status: active
governs: product
realized: crates/unica-coder/tests/daemon_process.rs::two_frontend_processes_race_to_one_daemon_pid_record_and_endpoint
supersedes: []
superseded-by: null
establishes: [INV.APP.HIDDEN-SERVICES]
design: docs/design/2026-08-23-v0-13-execution-surface-design.md
---

# Версионированный пользовательский daemon становится владельцем execution state

**Решение.** Скрытый daemon keyed протоколом и compile-time core ABI владеет
единственным durable InvocationStore и loopback endpoint для совместимых
frontend одного пользователя. Несовместимая core identity получает отдельный
endpoint. Endpoint публикуется в owner-only каталоге с случайным token, а idle
cleanup удаляет только запись точного владельца.

Обычный v0.12 stdio остаётся независим от daemon, пока следующий срез не
перенесёт Invocation routing. Workspace-keyed helpers сохраняются как
compatibility процессы до их последующего adapter migration.

**Почему.** Этот срез материализует process boundary и recovery ownership до
переключения публичного wire-контракта, не называя всю запланированную v0.13
поверхность реализованной.

**Цена.** Временно сосуществуют две внутренние топологии, hidden CLI и dormant
lazy-client seam; обе должны оставаться невидимыми обычному MCP frontend.
