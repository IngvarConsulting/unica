---
id: INV.APP.TASK-RETIREMENT
check:
  - crates/unica-coder/tests/daemon_receipt_ledger.rs::task_bind_direct_ack_and_receipt_terminal_expiry_release_exact_quota
  - crates/unica-coder/src/infrastructure/task_store_v5.rs::terminal_retirement_is_exact_explicit_and_reconciles_absence_after_uncertain_delete
---

# Очистка задания согласуется с его квитанцией

Очистке подлежит только завершённое задание после истечения срока,
отсчитанного от конечного результата. Перед удалением сохраняется
намерение очистить именно эту запись. После сбоя очистка продолжает это
намерение и различает удалённую запись, подтверждённое отсутствие,
неопределённый исход удаления и несовпадение идентичности.

Исчезновение активной задачи при оставшейся связи с квитанцией не считается
успешной очисткой: демон закрывает приём и требует перезапуска.
