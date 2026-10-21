---
id: INV.APP.DAEMON-INVOCATION-HANDOFF
check:
  - crates/unica-coder/tests/daemon_receipt_ledger.rs::cutoff_during_admission_projects_exact_unbound_task
  - crates/unica-coder/tests/daemon_receipt_ledger.rs::response_budget_is_not_receipt_identity
  - crates/unica-coder/src/interfaces/daemon_router.rs::live_daemon_hands_inline_work_over_the_cutoff_to_a_task_the_same_attempt_completes
  - crates/unica-coder/src/interfaces/daemon_router.rs::live_daemon_hands_a_failing_inline_attempt_over_the_cutoff_and_the_task_fails_once
  - crates/unica-coder/tests/platform/receipt_runtime.rs::committed_canonical_apply_and_saved_replay_keep_their_receipts_after_task_cancel
  - crates/unica-execution/src/invocation.rs::earlier_host_deadline_reserves_serialization_margin_without_using_timeout_as_prediction
  - crates/unica-coder/src/interfaces/daemon_router.rs::daemon_budget_is_what_remains_of_the_frontend_budget_once_the_connection_stands
gap: https://github.com/IngvarConsulting/unica/issues/980
---

# Затянувшийся прямой вызов продолжается в том же фоновом задании

У вызова, рассчитанного на прямой ответ, обычное окно ожидания — 7000 мс.
Если оно закончилось раньше операции, демон возвращает задание с заранее
выделенным `taskId`. Та же попытка продолжает работу; её успех или ошибка
становится конечным состоянием задания. Повторного исполнения нет.

Если срок наступил ещё при допуске к рабочему пространству, задание имеет
состояние `queued` и пока хранится в квитанции вызова. Запись в TaskStore
и идентичность актора появляются после связывания с актором.

Более короткий срок клиента может сократить окно. Повтор обращения к тому же
вызову не продлевает первоначальный срок и не создаёт новое задание.
Для работы, уже классифицированной как долгой, передача в задание выполняется
до долгой фазы, не дожидаясь семисекундного окна. Точное соотношение этой
передачи и начала работы ещё требует проверки, указанной в `gap`.

При нулевом бюджете прямого ответа вызов передаётся в сохраняемое задание
до начала предметной операции. Проверка этого порядка остаётся в `gap`.
