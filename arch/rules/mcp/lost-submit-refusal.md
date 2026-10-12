---
id: INV.WIRE.LOST-SUBMIT-TASK-ID
check:
  - crates/unica-coder/src/interfaces/daemon_router.rs::pending_receipt_beyond_the_frontend_cutoff_is_a_closed_refusal_not_a_retry
  - crates/unica-coder/src/interfaces/daemon_router.rs::recovery_window_closing_answers_the_closed_lost_submit_refusal
  - crates/unica-coder/src/interfaces/daemon_router.rs::recovery_without_a_receipt_whose_fence_meets_the_call_names_its_task
  - crates/unica-coder/src/interfaces/daemon_router.rs::recovery_without_a_receipt_fences_the_exact_key_and_answers_not_delivered
  - crates/unica-coder/src/interfaces/daemon_router.rs::a_call_whose_connection_never_stood_was_not_delivered
  - crates/unica-coder/src/interfaces/daemon_router.rs::native_clients_are_pointed_at_tasks_get_for_the_same_task
  - crates/unica-coder/src/interfaces/daemon_router.rs::lost_submit_names_its_task_and_a_retry_joins_it_instead_of_running_again
  - crates/unica-coder/src/interfaces/daemon_router.rs::a_read_whose_answer_was_lost_inline_is_readable_by_its_task_id
---

# Потерянный ответ на вызов называет задание, в котором его ждать

Если вызов дошёл до демона, а ответ на него потерян и не восстановлен
за окно восстановления, Unica отвечает ошибкой JSON-RPC `-32000`
с `data.code: submit_response_lost` и исходом `goElsewhere`. Ошибка содержит
`taskId`, зарезервированный для этого вызова, и `next` с ожиданием этого
задания: `unica.task.result` в режиме совместимости, `tasks/get` при
встроенных заданиях MCP. Тот же `taskId` повторён в тексте ошибки, потому что
не каждый хост показывает модели `data`. Других сведений ошибка не несёт:
`taskId` — случайный UUID, без путей, аргументов и текста демона.

По этому `taskId` демон отвечает состоянием вызова, пока он не стал
заданием: ещё исполняющийся вызов виден как `queued`, а завершившийся
прямым ответом, который никто не получил, — как завершённое задание с тем же
результатом. Предметная операция при этом не запускается повторно.
Такие сведения живут в памяти демона; после его перезапуска или вытеснения
давней записи `taskId` даёт `task_not_found`.

Если демон не видел вызова, frontend резервирует отмену для его точного
ключа ([правило ранней отмены](../runtime/receipt-cancel-reservation.md)).
Когда отмена сохранена, опоздавший кадр уже не запустится, и ответ —
`submit_not_delivered` с исходом `retry`, без `taskId`. Тот же код получает
вызов, кадр которого не ушёл: не установилось соединение, не прошло
рукопожатие или запись не началась. Если же отмена встретила квитанцию
вызова, ответ — `submit_response_lost` с его `taskId`.

Повтор вызова вместо ожидания остаётся новым вызовом. Для чтений его
поведение описано в [правиле присоединения](../runtime/read-call-joining.md).
Корпус приёмки не выражает потерю ответа демона: проверки исполняют
frontend с управляемым демоном и настоящий runtime v5 с потерянными ответами.
