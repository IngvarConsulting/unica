---
id: INV.APP.TASK-RETIREMENT-PROCESS-AUTHORITY
check:
  - crates/unica-coder/tests/platform/receipt_runtime.rs::reopened_retirement_rejects_old_authority_and_preserves_indexes_until_confirmed_delete
  - crates/unica-coder/tests/daemon_receipt_ledger.rs::task_bind_direct_ack_and_receipt_terminal_expiry_release_exact_quota
---

# Перезапуск отменяет прежнее право удалить задание

Демон не принимает право удаления завершённого задания, выданное предыдущим
процессом. Такой вызов не меняет задание, его связь с квитанцией и индексы.
Новый процесс сначала проверяет сохранённое намерение очистить именно это
задание и выдаёт новое право удаления.
