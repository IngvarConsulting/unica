---
id: INV.APP.DAEMON-INVOCATION-OWNERSHIP
check:
  - crates/unica-coder/tests/daemon_receipt_ledger.rs::exact_duplicate_preserves_cutoff_without_second_domain_callback
  - crates/unica-coder/tests/daemon_receipt_ledger.rs::crash_after_begun_returns_outcome_uncertain_without_replay
  - crates/unica-coder/tests/daemon_receipt_ledger.rs::side_effect_before_terminal_returns_outcome_uncertain_without_replay
  - crates/unica-coder/src/interfaces/daemon_router.rs::live_daemon_executes_once_and_compacts_the_acknowledged_receipt_to_a_tombstone
  - crates/unica-coder/src/interfaces/daemon_router.rs::live_daemon_hands_slow_work_off_to_a_task_that_get_wait_and_cancel_observe
  - crates/unica-coder/tests/daemon_receipt_ledger.rs::exact_duplicate_of_a_bound_task_reads_the_task_without_a_second_reservation
  - crates/unica-coder/src/infrastructure/daemon/runtime_v5/tests.rs::exact_repeat_submit_reads_its_bound_task_without_a_second_attempt
  - crates/unica-coder/src/infrastructure/daemon/runtime_v5/tests.rs::exact_repeat_racing_the_handoff_completion_reads_the_same_task
  - crates/unica-coder/src/infrastructure/daemon/runtime_v5/tests.rs::staged_handoff_completion_waits_for_the_key_lock
  - crates/unica-coder/src/infrastructure/daemon/runtime_v5/tests.rs::bound_handoff_completion_waits_for_the_key_lock
  - crates/unica-coder/src/infrastructure/daemon/runtime_v5/tests.rs::exact_repeat_of_a_retiring_task_is_not_found_and_reserves_nothing
  - crates/unica-coder/src/infrastructure/daemon/runtime_v5/tests.rs::exact_cancel_racing_the_handoff_completion_cancels_the_task
  - crates/unica-coder/src/infrastructure/daemon/runtime_v5/tests.rs::exact_cancel_during_the_begun_promotion_cancels_the_unstarted_task
  - crates/unica-coder/src/infrastructure/daemon/runtime_v5/tests.rs::exact_cancel_during_the_handoff_continuation_cancels_the_unstarted_task
  - crates/unica-coder/src/infrastructure/daemon/runtime_v5/tests.rs::exact_cancel_during_the_inline_cutoff_cancels_the_running_task
  - crates/unica-coder/src/infrastructure/daemon/runtime_v5/tests.rs::exact_cancel_marking_the_inline_handoff_cancels_the_running_task
  - crates/unica-coder/src/infrastructure/daemon/runtime_v5/tests.rs::exact_cancel_during_the_promised_continuation_cancels_the_unstarted_task
  - crates/unica-coder/src/infrastructure/daemon/runtime_v5/tests.rs::exact_cancel_racing_the_start_of_the_continued_task_ends_it_cancelled
  - crates/unica-coder/src/infrastructure/daemon/runtime_v5/tests.rs::task_id_cancel_during_the_begun_promotion_cancels_the_unstarted_task
  - crates/unica-coder/src/infrastructure/daemon/runtime_v5/tests.rs::task_id_cancel_during_the_handoff_continuation_cancels_the_unstarted_task
  - crates/unica-coder/src/infrastructure/daemon/runtime_v5/tests.rs::task_id_cancel_during_the_inline_cutoff_cancels_the_running_task
  - crates/unica-coder/src/infrastructure/daemon/runtime_v5/tests.rs::task_id_cancel_marking_the_inline_handoff_cancels_the_running_task
  - crates/unica-coder/src/infrastructure/daemon/runtime_v5/tests.rs::task_id_cancel_during_the_promised_continuation_cancels_the_unstarted_task
  - crates/unica-coder/src/infrastructure/daemon/runtime_v5/tests.rs::task_id_cancel_racing_the_start_of_the_continued_task_ends_it_cancelled
  - crates/unica-coder/src/infrastructure/daemon/runtime_v5/tests.rs::task_id_cancel_waits_for_the_staged_handoff_completion
  - crates/unica-coder/src/infrastructure/daemon/runtime_v5/tests.rs::begun_promotion_after_the_early_look_executes_the_materialized_task
---

# Повтор одного вызова не запускает работу заново

Пока хранится квитанция вызова или запись о её подтверждении, точная
идентичность вызова разрешает не более одного начала подготовки и исполнения. Повтор с той же идентичностью читает
сохранённое состояние; новый бюджет ответа не меняет ни результат, ни срок
передачи в фоновое задание. Чтение состояния задания и повторная отмена
не запускают предметную операцию снова.

Когда вызов передан в фоновое задание, его квитанцию хранит связь задания
с полным ключом вызова. Точный повтор, пришедший до или после завершения
задания, возвращает это же задание и не резервирует ключ заново. Вызов
с другим ключом и тем же `taskId` получает отказ несовпадения идентичности.
Отмена точного ключа или `taskId`, пришедшая во время передачи, достаётся
этому же заданию и не оставляет отдельной записи отмены. Если попытка ещё
не начала исполнение, такая отмена завершает задание как отменённое без
запуска предметной операции. Задание, которое демон создал при передаче
начатой попытки, исполняет эта же попытка, даже если она искала его
до того, как оно появилось. Если задание уже удаляется
по истечении срока хранения, повтор получает отказ «квитанция не найдена»
и тоже ничего не резервирует.

Гарантия ограничена сроком хранения задания. Когда задание и его связь
удалены полностью, следа вызова не остаётся: точный повтор после этого
резервирует ключ заново и исполняется как новый вызов.

Если процесс завершился после отметки о начале работы, но до надёжного
сохранения конечного результата, восстановление возвращает `outcome_uncertain`. Даже если внешнее
изменение уже произошло, автоматического повтора нет: общая транзакция между
этим изменением и квитанцией не предполагается.

Идентичность относится к конкретному вызову демона. Новый вызов MCP с теми же
аргументами не получает автоматически идентичность предыдущего.
