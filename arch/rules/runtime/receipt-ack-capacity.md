---
id: INV.APP.RECEIPT-ACK-CAPACITY
check:
  - crates/unica-coder/tests/daemon_receipt_ledger.rs::acknowledgement_crosses_former_tombstone_count_and_reopens_without_replay
  - crates/unica-coder/tests/daemon_receipt_ledger.rs::acknowledged_catalog_crosses_former_byte_pool_and_reopens_exactly
  - crates/unica-coder/tests/daemon_receipt_ledger.rs::deterministic_horizon_load_does_not_saturate
  - crates/unica-coder/src/infrastructure/receipt_ledger/tests.rs::tombstone_pool_does_not_consume_the_sixty_four_live_receipt_slots
  - crates/unica-coder/src/infrastructure/receipt_ledger/tests.rs::compact_tombstone_fits_512_bytes_at_the_maximum_valid_epoch_and_longest_tool_name
gap: https://github.com/IngvarConsulting/unica/issues/1119
---

# Накопленные подтверждения не препятствуют следующему ACK

Количество и суммарный размер сохранённых подтверждений не служат причиной
отказа в ACK. Они учитываются отдельно от активных квитанций и заданий и
не занимают резерв результата активного вызова.

Успешный ACK сохраняет точную идентичность вызова, хеш результата и время
первого подтверждения. Повторный ACK, чтение после открытия каталога и
повторная отправка того же вызова используют эти сведения без повторного
исполнения. Удаление подтверждений с истёкшим сроком выполняет механизм
[сроков хранения](receipt-acknowledgement.md).

Проверяются настоящий ACK после прежних 28864 записей, каталог с фактическим
суммарным размером больше прежних 14778368 байт, повторное открытие и граница
срока хранения. Предельная запись пока занимает до 512 байт; снятие этого
ограничения и границ одного batch остаётся отдельной частью `gap`.
