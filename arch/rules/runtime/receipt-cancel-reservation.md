---
id: INV.APP.RECEIPT-CANCEL-RESERVATION
check:
  - crates/unica-coder/src/infrastructure/receipt_ledger/tests.rs::cancel_reserved_persists_without_expiry_or_result_entitlement
  - crates/unica-coder/src/infrastructure/receipt_ledger/tests.rs::exact_submit_atomically_converts_cancel_reserved_to_full_cancelled_reservation
  - crates/unica-coder/src/infrastructure/receipt_ledger/tests.rs::mixed_legacy_and_persistent_cancel_rows_preserve_identity_across_reopen
  - crates/unica-coder/src/infrastructure/receipt_ledger/tests.rs::cancellation_schema_rejects_malformed_legacy_expiry_and_incompatible_v2_rows
  - crates/unica-coder/src/infrastructure/receipt_ledger/tests.rs::historical_cancel_deletion_releases_indexes_without_resurrection
  - crates/unica-coder/tests/daemon_receipt_ledger.rs::late_submit_preserves_early_cancellation_after_delay_and_restart
  - crates/unica-coder/tests/daemon_receipt_ledger.rs::cancel_before_submit_preserves_full_key_after_arbitrary_delay
  - crates/unica-coder/tests/daemon_receipt_ledger.rs::cancel_reserved_reopens_and_recovery_preserves_original_cancellation
---

# Отмена, пришедшая раньше вызова, сохраняется для его точного ключа

ReceiptLedger сохраняет раннюю отмену без срока действия. Повтор отмены,
восстановление и повторное открытие хранилища сохраняют исходную запись.
Она не резервирует место под результат.

Когда соответствующий вызов приходит, его запись получает сохранённый признак
отмены и резерв результата одним переходом. Предметный код не запускается.
Отмена относится к полному точному ключу вызова; похожий идентификатор не даёт
права удалить её или отменить другой вызов.

Старые записи отмены читаются с проверкой прежнего формата, но сохранённый
в них срок больше не снимает отмену. Уже зафиксированные старой версией
свидетельства удаления завершают восстановление без воскрешения удалённой записи.

Проверки подтверждают файловое хранение, восстановление и преобразование записи.
Маршрут публичной отмены до появления задания рассматривается отдельно в
[issue929](https://github.com/IngvarConsulting/unica/issues/929).
