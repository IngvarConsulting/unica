---
id: INV.APP.DAEMON-TERMINAL-RECONCILIATION
status: active
governs: product
decision: DEC.2026-09-07.DAEMON-V5-PRODUCTION-CUTOVER
check:
  - crates/unica-coder/src/infrastructure/task_store_v5.rs::completed_terminal_cas_reconciles_commit_uncertain_by_exact_readback
  - crates/unica-coder/src/infrastructure/task_store_v5.rs::post_publish_sync_failure_is_commit_uncertain_with_exact_visible_readback
  - crates/unica-coder/tests/daemon_receipt_ledger.rs::task_terminal_receipt_crash_reconciles_without_replay
  - crates/unica-coder/tests/daemon_receipt_ledger.rs::every_cross_store_crash_point_reconciles_without_split_brain
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

Единый absolute terminal deadline захватывается до result preparation/clone,
Arc allocation, thread scheduling и store-channel send. Counting serialization,
store actor wait, file serialization и retry используют его без reset; worker,
начавший после deadline, не вызывает store и переводит daemon в fail-stop.
