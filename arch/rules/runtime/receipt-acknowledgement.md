---
id: INV.APP.RECEIPT-ACKNOWLEDGEMENT
check:
  - crates/unica-coder/src/infrastructure/receipt_ledger/tests/crash_process_tests.rs::ack_crash_after_compact_row_rename_reopens_the_same_tombstone
  - crates/unica-coder/src/infrastructure/receipt_ledger/tests/crash_process_tests.rs::ack_crash_after_witness_row_before_generation_heals_and_compacts_on_reopen
  - crates/unica-coder/src/infrastructure/receipt_ledger/tests.rs::rejected_ack_does_not_reclaim_an_unrelated_expired_tombstone
  - crates/unica-coder/src/interfaces/daemon_router.rs::live_daemon_executes_once_and_compacts_the_acknowledged_receipt_to_a_tombstone
  - crates/unica-coder/src/interfaces/daemon_router.rs::malformed_direct_receipt_is_recovered_by_key_and_only_the_strict_one_is_acknowledged
gap: https://github.com/IngvarConsulting/unica/issues/1119
---

# Подтверждение прямого ответа освобождает результат и сохраняет след вызова

Подтверждение принимается только для конечной прямой квитанции с совпавшими
ключом и хешем результата. Преждевременное подтверждение или другой хеш
не меняют запись.

После подтверждения полное содержимое удаляется. Остаётся компактная запись
с ключом, хешем и временем первого подтверждения. Повтор подтверждения
возвращает те же сведения; повтор вызова получает эту запись и не запускает
операцию заново. Компактная запись восстанавливается после повторного
открытия хранилища с теми же ключом, хешем и первым временем подтверждения.

Само истечение времени не удаляет неподтверждённый прямой результат или
компактную запись и не освобождает их учтённое место. Ограничивающий срок
подчиняется [правилу таймеров](timer-purpose.md) и сохраняет принятую работу.

Проверки подтверждают допуск ACK, сжатие результата, восстановление
компактной записи и отсутствие повторного исполнения. Сохранность результата
и следа вызова за действующими сроками хранения остаётся в `gap`.
