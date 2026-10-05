---
id: INV.APP.RECEIPT-RECOVERY-BOUNDS
check:
  - crates/unica-coder/src/infrastructure/receipt_ledger/tests.rs::live_receipts_cross_former_count_and_byte_quotas_without_losing_exact_reservations
  - crates/unica-coder/src/infrastructure/receipt_ledger/tests.rs::batched_live_receipts_and_actor_snapshot_cross_former_quotas_and_reopen
  - crates/unica-coder/src/infrastructure/receipt_ledger/tests.rs::expired_recovery_deadline_fails_before_staging_cleanup_or_catalog_mutation
  - crates/unica-coder/src/infrastructure/receipt_ledger/tests.rs::oversized_live_persisted_row_is_corruption_and_fail_stops_the_store
gap: https://github.com/IngvarConsulting/unica/issues/1119
---

# Восстановление проверяет весь сохранённый каталог

Открытие хранилища читает все его записи без численной квоты на число файлов.
Повреждённая опубликованная запись не становится отсутствующим вызовом
или произвольно выбранным результатом. Демон не принимает работу до успешной
проверки и согласования хранилищ.

Незавершённая временная запись сама по себе не доказывает совершённый переход.
Её очистка разрешена только после проверки сохранённых данных и принадлежности
временного файла.

Повторное открытие проверено за прежней границей общего пула, в том числе
после пакетной записи. Проверки отдельных записей и отказ до очистки
при прерванном восстановлении сохраняются. Оставшиеся технические пределы
размера отдельной записи и времени восстановления снимаются по согласованному
решению в `gap`; они не ограничивают число восстанавливаемых записей.
