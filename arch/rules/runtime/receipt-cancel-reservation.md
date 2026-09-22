---
id: INV.APP.RECEIPT-CANCEL-RESERVATION
check:
  - crates/unica-coder/src/infrastructure/receipt_ledger/tests.rs::cancel_reserved_persists_exact_absolute_expiry_without_result_entitlement
  - crates/unica-coder/src/infrastructure/receipt_ledger/tests.rs::exact_submit_atomically_converts_cancel_reserved_to_full_cancelled_reservation
---

# Отмена, пришедшая раньше вызова, сохраняется для его точного ключа

ReceiptLedger хранит такую отмену 7125 мс. Повтор отмены и повторное открытие
хранилища не продлевают срок. Запись не резервирует место под результат.

Если соответствующий вызов приходит до истечения срока, его запись получает
сохранённый признак отмены и резерв результата одним переходом. Это не отмена
другого вызова с похожим идентификатором.

Проверки подтверждают файловое хранение и преобразование записи. Маршрут
публичной отмены до появления задания рассматривается отдельно в
[issue929](https://github.com/IngvarConsulting/unica/issues/929).
