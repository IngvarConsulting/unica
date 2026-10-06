---
id: INV.APP.ADMISSION-FAILURE-ISOLATION
check:
  - crates/unica-coder/src/infrastructure/daemon/runtime_v5/tests.rs::root_view_keeps_the_same_task_and_daemon_past_the_former_deadlines
  - crates/unica-coder/src/infrastructure/daemon/runtime_v5/tests.rs::root_check_completes_the_whole_inspection_in_its_task
  - crates/unica-coder/src/infrastructure/daemon/runtime_v5/tests.rs::cancel_during_actor_admission_after_handoff_never_begins_and_keeps_the_daemon
  - crates/unica-coder/src/infrastructure/daemon/runtime_v5/tests.rs::cancel_during_inline_admission_before_handoff_publishes_cancelled
  - crates/unica-coder/src/infrastructure/daemon/runtime_v5/tests.rs::noncooperative_cancel_keeps_only_its_task_while_the_daemon_serves_others
  - crates/unica-coder/src/infrastructure/daemon/runtime_v5/tests.rs::promoted_reads_run_past_former_deadlines_and_complete_through_their_tasks
  - crates/unica-coder/tests/daemon_receipt_ledger.rs::unbound_validation_and_admission_continue_past_the_former_fail_stop_grace
  - crates/unica-coder/tests/daemon_receipt_ledger.rs::noncooperative_prepare_cancel_keeps_the_daemon_listening
---

# Затянувшееся задание держит только себя, а не весь демон

Срок ожидания клиента ограничивает прямой ответ, а не работу задания.
Проверка аргументов, допуск к рабочему пространству, подготовка и исполнение
идут до конечного исхода и после передачи вызова в задание. Отдельного
срока у них нет, и медленное задание не останавливает демон.

Явная отмена вызова или задания останавливает незавершённый допуск
у ближайшей контрольной точки: операция не начинается, вызов или задание
завершается отменой.
Исполнитель, который не ответил на отмену, держит только своё задание:
у него виден запрос отмены, а соседние задания и демон продолжают работать.
Пока такой исполнитель не вернулся, его задание считается активной работой:
демон не завершается по простою.
Завершение дерева процессов по отмене сохраняется по
[правилу жизненного цикла](../platform/process-tree-lifecycle.md).

После отмены или отказа задержанный исполнитель не публикует результат
вместо отмены. Начатое изменение информационной базы ведёт себя по
[правилу отмены изменения](runner-mutation-cancellation.md).
[Неопределённая запись в хранилище](receipt-store-fail-stop.md)
по-прежнему требует защитной остановки.
