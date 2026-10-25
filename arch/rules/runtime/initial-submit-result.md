---
id: INV.WIRE.DAEMON-INITIAL-SUBMIT-RESULT
check:
  - crates/unica-coder/src/interfaces/daemon_router.rs::a_first_submit_is_answered_directly_or_by_its_task_never_pending
  - crates/unica-coder/src/interfaces/daemon_router.rs::a_first_submit_answered_receipt_pending_is_a_closed_refusal_without_resubmit
  - crates/unica-coder/src/interfaces/daemon_router.rs::live_daemon_hands_inline_work_over_the_cutoff_to_a_task_the_same_attempt_completes
  - crates/unica-coder/src/interfaces/daemon_router.rs::pending_receipt_is_polled_until_its_original_budget_settles
  - crates/unica-coder/src/interfaces/daemon_router.rs::pending_receipt_beyond_the_frontend_cutoff_is_a_closed_refusal_not_a_retry
  - crates/unica-coder/src/interfaces/daemon_router.rs::a_fenced_frame_never_runs_and_a_held_one_is_answered_by_its_state
---

# Первый submit получает результат, задание или отказ

Обычная первая отправка вызова не возвращает `receipt_pending` вместо
результата, задания или отказа. Если работа не завершилась в окне прямого
ответа, её продолжение представляет задание.

Ответ `receipt_pending` допустим при проверке ещё исполняющейся квитанции
после потери ответа или при её отмене. Он не даёт нового срока исполнения
и не запускает предметную операцию повторно. Если первая отправка всё же
получила `receipt_pending`, frontend отвечает закрытым отказом
`receipt_pending` без повторной отправки.
