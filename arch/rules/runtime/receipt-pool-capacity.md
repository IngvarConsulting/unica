---
id: INV.APP.RECEIPT-POOL-CAPACITY
check:
  - crates/unica-coder/src/infrastructure/receipt_ledger/tests.rs::live_receipts_cross_former_count_and_byte_quotas_without_losing_exact_reservations
  - crates/unica-coder/src/infrastructure/receipt_ledger/tests.rs::early_cancellations_cross_the_former_live_quota_without_reserving_results
  - crates/unica-coder/src/infrastructure/receipt_ledger/tests.rs::submit_admission_preserves_unrelated_cancellations_past_the_former_live_quota
  - crates/unica-coder/src/infrastructure/receipt_ledger/tests.rs::batched_live_receipts_and_actor_snapshot_cross_former_quotas_and_reopen
  - crates/unica-coder/src/infrastructure/receipt_ledger/tests.rs::live_catalog_arithmetic_overflow_rejects_single_and_batch_mutations_before_commit
  - crates/unica-coder/tests/daemon_receipt_ledger.rs::cancel_reserved_crosses_former_live_quotas_without_result_reservation
---

# Число и суммарный размер активных квитанций не ограничивают приём

Активные и неподтверждённые квитанции принимаются без численной квоты
на число записей или сумму фактических байтов и резервов результатов.
Новый вызов не вытесняет чужие квитанции ради своего приёма.
После перезапуска каталог восстанавливает все принятые записи; его проверка
и снимок не вводят прежние квоты повторно.

Ранняя отмена сохраняет точную идентичность вызова и не резервирует результат.
Обычная квитанция сохраняет учёт фактических байтов и принадлежащего ей
резерва. Переполнение счётчика, противоречие индексов или повреждение
хранилища остаются ошибками; проверки выполняются до фиксации изменений.

По решению владельца в #1119 снимаются также остальные квоты и автоматические
сроки. Этот шаг меняет общий пул активных квитанций. Ограничения отдельных
результатов, ACK-записей, TaskStore и сроки их хранения разбираются следующими
целыми путями в [#1119](https://github.com/IngvarConsulting/unica/issues/1119).
