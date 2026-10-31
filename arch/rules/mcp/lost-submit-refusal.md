---
id: INV.WIRE.LOST-SUBMIT-TASK-ID
check:
  - crates/unica-coder/src/interfaces/daemon_router.rs::pending_receipt_beyond_the_frontend_cutoff_is_the_reserved_task_not_a_retry
  - crates/unica-coder/src/interfaces/daemon_router.rs::recovery_window_closing_answers_the_task_reserved_for_the_call
  - crates/unica-coder/src/interfaces/daemon_router.rs::recovery_without_a_receipt_whose_fence_meets_the_call_names_its_task
  - crates/unica-coder/src/interfaces/daemon_router.rs::recovery_without_a_receipt_fences_the_exact_key_and_answers_not_delivered
  - crates/unica-coder/src/interfaces/daemon_router.rs::a_call_whose_connection_never_stood_was_not_delivered
  - crates/unica-coder/src/interfaces/daemon_router.rs::readiness_timeout_before_outer_cutoff_is_not_delivered
  - crates/unica-daemon/src/client_v5.rs::failed_readiness_kills_the_child_within_the_original_budget
  - crates/unica-daemon/src/client_v5.rs::startup_handshake_timeout_is_expiry_but_early_rejection_is_not
  - crates/unica-daemon/src/client_v5.rs::failed_readiness_preserves_expiry_when_cleanup_cannot_confirm_exit
  - crates/unica-coder/src/interfaces/daemon_router.rs::a_daemon_refusing_the_connection_needs_a_person_not_a_repeat
  - crates/unica-coder/src/interfaces/daemon_router.rs::a_lost_submit_is_the_queued_task_reserved_for_the_call
  - crates/unica-coder/src/interfaces/daemon_router.rs::a_lost_task_the_daemon_does_not_know_answers_task_not_found
  - crates/unica-coder/src/interfaces/mcp.rs::a_lost_submit_is_a_task_for_native_clients_and_a_task_receipt_for_others
  - crates/unica-coder/src/interfaces/daemon_router.rs::lost_submit_names_its_task_and_a_retry_joins_it_instead_of_running_again
  - crates/unica-coder/src/interfaces/daemon_router.rs::a_read_whose_answer_was_lost_inline_is_readable_by_its_task_id
  - crates/unica-coder/src/interfaces/daemon_router.rs::the_task_id_of_a_call_still_inline_can_be_cancelled
  - crates/unica-coder/src/interfaces/daemon_router.rs::a_fenced_frame_never_runs_and_a_held_one_is_answered_by_its_state
  - crates/unica-execution/src/receipt_runtime/joining.rs::a_fence_stays_bounded_and_ends_after_its_retention
gap: https://github.com/IngvarConsulting/unica/issues/1743
---

# Потерянный ответ на вызов называет задание, в котором его ждать

Если вызов дошёл до демона, а ответ на него потерян и не восстановлен
за окно восстановления, вызов может продолжаться как задание,
зарезервированное под его `taskId`. Клиент получает это задание так же,
как при обычной передаче вызова в задание. Клиент со встроенными
заданиями MCP получает `CreateTaskResult` со статусом `queued` и опрашивает
задание через `tasks/get`. Остальные клиенты получают обычную квитанцию
задания: `data.task` с этим `taskId` и статусом `queued` и `next`
с `unica.task.result`. Квитанция не несёт других сведений: `taskId` —
случайный UUID, без путей, аргументов и текста демона. Если демон так
и не принял вызов к исполнению или больше его не знает,
`unica.task.result` по этому `taskId` отвечает `task_not_found`.

По этому `taskId` демон отвечает состоянием вызова, пока он не стал
заданием: ещё исполняющийся вызов виден как `queued` и отменяется через
`unica.task.cancel` или `tasks/cancel`, а завершившийся прямым ответом,
который никто не получил, — как завершённое задание с тем же результатом.
Предметная операция при этом не запускается повторно. У принятого
самостоятельного вызова этот lookup сохраняется после перезапуска в своей
области сборки по [правилу выдачи](../runtime/execution-delivery.md). Это
не превращает непринятый кадр или временный alias в долговечную попытку.

Если демон не видел вызова, frontend ставит преграду для его точного ключа.
Кадр вызова читает только процесс демона, которому он отправлен, поэтому
преграда живёт в памяти этого процесса десять минут — с большим запасом
против бюджета вызова в 7 с. Пока она стоит, опоздавший кадр не запускается,
и ответ — `submit_not_delivered` с исходом `retry`, без `taskId`. Если
демон уже держит вызов, он отвечает его состоянием, и ответ — квитанция
задания с его `taskId`. Записи ранней отмены
([правило](../runtime/receipt-cancel-reservation.md)) преграда не создаёт.

Вызов, кадр которого не ушёл, также получает `submit_not_delivered`, если
истёк срок ожидания соединения, в том числе готовности запущенного демона.
Оставшийся запас времени на очистку не меняет этот исход. Если попытка
соединения получила явный отказ до истечения срока, ответ `task_backend_failed`
с `detailCode: backend_incompatible` и исходом `needsHuman`. К таким отказам
относятся ошибка запуска, завершение демона до готовности, отказ рукопожатия
и несовпадение ядра. Повтор того же вызова ничего не изменит.

Повтор вызова вместо ожидания остаётся новым вызовом. Для чтений его
поведение описано в [правиле присоединения](../runtime/read-call-joining.md).
Корпус приёмки не выражает потерю ответа демона: проверки исполняют
frontend с управляемым демоном и настоящий runtime v5 с потерянными ответами.

Нынешний early lookup живёт в памяти и может дать `task_not_found` после
перезапуска. Проверки подтверждают доступ в живом процессе и semantic fence;
долговечность принятого lookup и права выдачи остаются в `gap`.
