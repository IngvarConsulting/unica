---
id: INV.APP.DAEMON-TERMINAL-RECONCILIATION
check:
  - crates/unica-coder/src/infrastructure/daemon/runtime_v5/tests.rs::startup_completes_exact_staged_terminal_after_post_store_crash_without_replay
  - crates/unica-coder/src/infrastructure/daemon/runtime_v5/tests.rs::startup_completes_exact_staged_receipt_after_terminal_link_commit_without_replay
  - crates/unica-coder/src/infrastructure/daemon/runtime_v5/tests.rs::startup_publishes_saved_staged_winner_from_exact_provisional_without_replay
  - crates/unica-coder/src/infrastructure/daemon/runtime_v5/tests.rs::startup_refuses_changed_staged_winner_metadata_before_any_store_mutation
  - crates/unica-coder/src/infrastructure/daemon/runtime_v5/tests.rs::startup_validates_all_staged_owners_before_completing_any_receipt_or_link
  - crates/unica-coder/tests/daemon_receipt_ledger.rs::begun_staged_post_store_crash_recovers_exact_winner_without_replay
  - crates/unica-coder/src/infrastructure/task_store_v5.rs::completed_terminal_cas_reconciles_commit_uncertain_by_exact_readback
  - crates/unica-coder/src/infrastructure/task_store_v5.rs::terminal_cas_rejects_foreign_stale_invalid_state_and_different_winner
  - crates/unica-coder/tests/daemon_receipt_ledger.rs::task_terminal_receipt_crash_reconciles_without_replay
  - crates/unica-coder/tests/daemon_receipt_ledger.rs::every_cross_store_crash_point_reconciles_without_split_brain
  - crates/unica-coder/tests/daemon_receipt_ledger.rs::unstaged_task_bind_is_refused_against_a_staged_handoff_predecessor
  - crates/unica-coder/tests/daemon_receipt_ledger.rs::prepared_handoff_preserves_staged_epoch_across_clock_jumps
  - crates/unica-coder/tests/daemon_receipt_ledger.rs::promised_queued_staged_transfer_preserves_exact_winner_at_same_epoch
  - crates/unica-coder/tests/daemon_receipt_ledger.rs::promised_queued_staged_transfer_preserves_exact_winner_across_clock_jumps
  - crates/unica-coder/src/infrastructure/task_store_v5.rs::staged_terminal_retry_refuses_foreign_successor_without_rewriting_bytes
  - crates/unica-coder/src/infrastructure/task_store_v5.rs::staged_terminal_retry_refuses_changed_successor_metadata_without_rewriting_bytes
  - crates/unica-coder/src/infrastructure/task_store_v5.rs::staged_terminal_cas_reconciles_uncertain_commit_by_exact_successor
---

# Сохранённый результат задачи восстанавливается без повторного исполнения

Если запись конечного состояния дала неопределённый исход, точный повтор
может подтвердить уже сохранённый результат чтением. Он не создаёт новую
версию записи. При новом переходе требуются ожидаемые идентичность, версия
и исходное состояние; чужая запись или другой конечный результат отклоняются.

Перенос подготовленного результата из квитанции проверяет сохранённое
доказательство передачи и полную исходную запись TaskStore. Этот переход
допустим из `Queued` и `Working`; обычное завершение исполняемой задачи
сохраняет требование `Working`. Точный повтор переноса принимает только
полную ожидаемую запись после перехода: конечное состояние, следующую
версию, сохранённое время результата и все неизменяемые поля исходной записи.

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
