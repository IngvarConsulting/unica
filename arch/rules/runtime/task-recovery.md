---
id: INV.APP.DAEMON-TASK-RECOVERY
check:
  - crates/unica-coder/src/infrastructure/task_store_v5.rs::recovery_terminalizes_queued_without_starting_domain_work
  - crates/unica-coder/src/infrastructure/task_store_v5.rs::recovered_begun_task_is_created_working_and_keeps_cancel_intent
  - crates/unica-coder/src/infrastructure/daemon/runtime_v5/tests.rs::startup_terminalizes_pre_task_receipts_without_replaying_domain_work
  - crates/unica-coder/src/infrastructure/daemon/runtime_v5/tests.rs::startup_materializes_handoff_without_replaying_begun_work
  - crates/unica-coder/tests/daemon_receipt_ledger.rs::restart_begun_without_committed_handoff_is_direct_outcome_uncertain
  - crates/unica-coder/tests/daemon_receipt_ledger.rs::working_readback_before_receipt_begun_recovers_interrupted_without_callback
---

# Восстановление демона завершает оставшиеся задания без повторного исполнения

При запуске демон сначала читает существующие записи задач без изменения
их состояний. Затем сверяет их с квитанциями вызовов и завершает восстановление
до начала приёма запросов. Хранилище прежнего протокола не переиспользуется.

Если работа ещё не начиналась, восстановление фиксирует `interrupted`
или уже принятую отмену. Если начало работы отмечено, а надёжно сохранённого
конечного результата нет, фиксируется `outcome_uncertain`, даже при запросе отмены.
Предметная операция не запускается снова.

Сохранённое намерение передать вызов в фоновое задание восстанавливает это
же задание и его признак отмены. Оно получает конечное состояние по тем же
условиям; оставлять его навсегда в `working` или изображать возобновление
работы нельзя.
