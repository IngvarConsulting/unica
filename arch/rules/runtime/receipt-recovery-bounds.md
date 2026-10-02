---
id: INV.APP.RECEIPT-RECOVERY-BOUNDS
check:
  - crates/unica-coder/src/infrastructure/receipt_ledger/tests.rs::expired_recovery_deadline_fails_before_staging_cleanup_or_catalog_mutation
  - crates/unica-coder/src/infrastructure/receipt_ledger/tests.rs::oversized_live_persisted_row_is_corruption_and_fail_stops_the_store
gap: https://github.com/IngvarConsulting/unica/issues/985
---

# Восстановление квитанций ограничено по объёму и времени

Открытие хранилища ограничивает число читаемых файлов, размер записей
и общий срок восстановления. Повреждённая опубликованная запись хранилища
не становится отсутствующим вызовом или произвольно выбранным результатом.
Демон не принимает работу до успешной проверки и согласования хранилищ.

Незавершённая временная запись сама по себе не доказывает совершённый переход.
Её очистка разрешена только после проверки сохранённых данных и принадлежности
временного файла.

Проверены отказ до очистки при истёкшем сроке и остановка владельца при чтении
слишком большой сохранённой записи. Полные границы открытия и восстановления
описаны в `gap`.
