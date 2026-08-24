---
id: INV.APP.DAEMON-TERMINAL-RECONCILIATION
status: active
governs: product
decision: DEC.2026-08-24.DAEMON-INVOCATION-ROUTING-SLICE
check: crates/unica-coder/src/application/invocation.rs::terminal_publication_faults_reconcile_without_reexecution_or_false_idle
scope: [app, cache]
---

# Durable terminal подтверждается без повторного domain execution

Materialized Task атомарно создаётся как `Working`. Любой `Ok(create)` до
execution сравнивается с ожидаемыми TaskId, invocation/tool/digest identities,
полной формой state и согласованными timestamps. Неопределённый create,
complete, fail или cancel commit подтверждается чтением точной ожидаемой
identity и state. TaskId выделяется и live owner регистрируется до create;
domain execution начинается только после точного durable подтверждения
начального `Working`.

Reconciliation имеет абсолютный monotonic budget и bounded exponential
backoff. Пока подтверждение возможно, daemon сохраняет live owner и opaque
actor capability, не разрешает idle exit и повторяет только store operation,
но никогда не domain execution. Если точный commit нельзя доказать в пределах
policy, executor закрывает новые submit состоянием `RestartRequested`, не
публикует staged result и просит процесс завершиться без ожидания зависших
worker/execution threads. Только смерть PID является освобождением этих
ресурсов; следующее открытие store закрывает оставшийся `Working` как
interrupted.
Повторный cancel и cancel, проигравший complete/fail/cancel, возвращают точное
победившее durable terminal-состояние. Get/wait только наблюдают durable record
и не запускают работу.
