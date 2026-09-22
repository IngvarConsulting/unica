---
id: INV.APP.RECEIPT-ACK-CAPACITY
check:
  - crates/unica-coder/src/infrastructure/receipt_ledger/tests.rs::tombstone_pool_does_not_consume_the_sixty_four_live_receipt_slots
  - crates/unica-coder/src/infrastructure/receipt_ledger/tests.rs::compact_tombstone_fits_512_bytes_at_the_maximum_valid_epoch_and_longest_tool_name
gap: https://github.com/IngvarConsulting/unica/issues/985
---

# Заполненный пул подтверждений не уничтожает прямой результат

Сведения о подтверждённых ответах учитываются отдельно от активных квитанций
и заданий: до 28864 записей, до 512 байт каждая, всего до 14778368 байт.
Они не занимают место, зарезервированное для результата активного вызова.

При нехватке места удаляются только записи с истёкшим сроком. Если этого
недостаточно, подтверждение возвращает `tombstone_capacity`, а исходный
неподтверждённый результат и его резерв сохраняются. Это не успешное
подтверждение и не основание остановить демон.

Проверены раздельный учёт на подготовленном каталоге и кодирование одной
предельной записи. Полное переполнение и сохранение результата через
production ACK ещё требуют проверки, указанной в `gap`.
