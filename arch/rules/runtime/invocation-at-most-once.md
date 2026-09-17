---
id: INV.APP.DAEMON-INVOCATION-OWNERSHIP
check:
  - crates/unica-coder/tests/daemon_receipt_ledger.rs::exact_duplicate_preserves_cutoff_without_second_domain_callback
  - crates/unica-coder/tests/daemon_receipt_ledger.rs::crash_after_begun_returns_outcome_uncertain_without_replay
  - crates/unica-coder/tests/daemon_receipt_ledger.rs::side_effect_before_terminal_returns_outcome_uncertain_without_replay
  - crates/unica-coder/src/interfaces/daemon_router.rs::live_daemon_executes_once_and_compacts_the_acknowledged_receipt_to_a_tombstone
  - crates/unica-coder/src/interfaces/daemon_router.rs::live_daemon_hands_slow_work_off_to_a_task_that_get_wait_and_cancel_observe
---

# Повтор одного вызова не запускает работу заново

Пока хранится квитанция вызова или запись о её подтверждении, точная
идентичность вызова разрешает не более одного начала подготовки и исполнения. Повтор с той же идентичностью читает
сохранённое состояние; новый бюджет ответа не меняет ни результат, ни срок
передачи в фоновое задание. Чтение состояния задания и повторная отмена
не запускают предметную операцию снова.

Если процесс завершился после отметки о начале работы, но до надёжного
сохранения конечного результата, восстановление возвращает `outcome_uncertain`. Даже если внешнее
изменение уже произошло, автоматического повтора нет: общая транзакция между
этим изменением и квитанцией не предполагается.

Идентичность относится к конкретному вызову демона. Новый вызов MCP с теми же
аргументами не получает автоматически идентичность предыдущего.
