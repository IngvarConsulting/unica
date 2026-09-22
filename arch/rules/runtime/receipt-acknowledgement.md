---
id: INV.APP.RECEIPT-ACKNOWLEDGEMENT
check:
  - crates/unica-coder/src/infrastructure/receipt_ledger/tests.rs::direct_ack_compacts_payload_to_restart_stable_idempotent_tombstone
  - crates/unica-coder/src/infrastructure/receipt_ledger/tests.rs::acknowledged_tombstone_is_physically_reclaimed_at_its_absolute_expiry
  - crates/unica-coder/tests/daemon_receipt_ledger.rs::acknowledge_receipt_compacts_to_bounded_tombstone
  - crates/unica-coder/tests/daemon_receipt_ledger.rs::direct_unacked_expiry_deletes_payload_and_releases_exact_quota
---

# Подтверждение прямого ответа освобождает результат и сохраняет след вызова

Подтверждение принимается только для конечной прямой квитанции с совпавшими
ключом и хешем результата. Преждевременное подтверждение или другой хеш
не меняют запись.

После подтверждения полное содержимое удаляется. Остаётся компактная запись
с ключом, хешем и временем первого подтверждения. Повтор подтверждения
возвращает те же сведения; повтор вызова получает эту запись и не запускает
операцию заново. Компактная запись хранится 15 минут от первого подтверждения.
Повторное подтверждение этот срок не продлевает. По истечении срока запись удаляется.

Неподтверждённый прямой результат хранится час. По истечении срока удаляются
его байты и резерв места; после перезапуска этот результат не восстанавливается.
