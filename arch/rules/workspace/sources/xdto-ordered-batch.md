---
id: INV.SOURCE.XDTO-ORDERED-BATCH
check:
  - crates/unica-coder/src/infrastructure/native_operations/xdto.rs::xdto_writer_orchestration_reports_exact_and_conflicting_duplicates_without_bytes
  - crates/unica-coder/src/infrastructure/native_operations/xdto.rs::staged_xdto_add_type_then_property_reads_prior_postimage
  - crates/unica-coder/src/infrastructure/native_operations/xdto.rs::staged_xdto_poisoned_second_op_publishes_nothing
  - crates/unica-coder/src/infrastructure/native_operations/xdto.rs::staged_xdto_dry_and_real_share_postimage_effects_and_revision
---

# Изменения пакета XDTO выполняются по порядку одной публикацией

Операция над пакетом XDTO видит результат предыдущих операций того же
запроса. Например, можно создать тип и сразу добавить ему свойство.
Ошибка последующей операции не оставляет частично записанный пакет
и указывает положение ошибочной операции.

Предпросмотр и применение строят одинаковый итоговый текст и события.
Предпросмотр сохраняет исходный файл; применение публикует пакет один раз.
Проверки проходят планировщик XDTO и публикацию через актора.

Точное повторное добавление не меняет пакет; добавление с тем же именем
и иным содержимым отклоняется. Проверка дублей проходит внутренний писатель.
