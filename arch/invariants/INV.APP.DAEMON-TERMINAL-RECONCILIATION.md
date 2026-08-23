---
id: INV.APP.DAEMON-TERMINAL-RECONCILIATION
status: active
governs: product
decision: DEC.2026-08-24.DAEMON-INVOCATION-ROUTING-SLICE
check: crates/unica-coder/src/application/invocation.rs::terminal_publication_faults_reconcile_without_reexecution_or_false_idle
scope: [app, cache]
---

# Durable terminal подтверждается без повторного domain execution

Materialized Task атомарно создаётся как `Working`. Неопределённый create,
complete, fail или cancel commit подтверждается чтением точной ожидаемой
identity и state. Пока terminal не подтверждён, daemon сохраняет live owner и
opaque actor capability, не разрешает idle exit и повторяет только публикацию,
но никогда не domain execution. Повторный cancel подтверждает то же закрытое
terminal-состояние; get/wait только наблюдают durable record и не запускают
работу.
