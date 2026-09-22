---
id: INV.APP.DAEMON-TERMINAL-RECONCILIATION
check:
  - crates/unica-coder/src/infrastructure/task_store_v5.rs::completed_terminal_cas_reconciles_commit_uncertain_by_exact_readback
  - crates/unica-coder/src/infrastructure/task_store_v5.rs::terminal_cas_rejects_foreign_stale_invalid_state_and_different_winner
  - crates/unica-coder/tests/daemon_receipt_ledger.rs::task_terminal_receipt_crash_reconciles_without_replay
  - crates/unica-coder/tests/daemon_receipt_ledger.rs::every_cross_store_crash_point_reconciles_without_split_brain
  - crates/unica-coder/tests/daemon_receipt_ledger.rs::unstaged_task_bind_is_refused_against_a_staged_handoff_predecessor
---

# Сохранённый результат задачи восстанавливается без повторного исполнения

Если запись конечного состояния дала неопределённый исход, точный повтор
может подтвердить уже сохранённый результат чтением. Он не создаёт новую
версию записи. При новом переходе требуются ожидаемые идентичность, версия
и исходное состояние; чужая запись или другой конечный результат отклоняются.

Сбой между сохранением результата и обновлением связи с квитанцией
не теряет результат. При восстановлении записи сводятся к одному
`taskId` и одному конечному состоянию. Уже сохранённый подготовленный
результат переносится без изменений; предметная операция не исполняется снова.

Если надёжно сохранённого результата нет, действует
[правило восстановления незавершённых задач](task-recovery.md).

Подготовленный конечный результат удаляется из квитанции только после
точного подтверждения соответствующей конечной записи TaskStore.
Для ещё не завершённой операции достаточно подтверждения незавершённой
записи; этим путём нельзя передавать уже подготовленный конечный результат.
