---
id: INV.SOURCE.XDTO-ORDERED-BATCH
check:
  - crates/unica-coder/src/infrastructure/native_operations/xdto.rs::staged_xdto_exact_repeat_of_an_addition_plans_and_publishes_nothing
  - crates/unica-coder/src/infrastructure/native_operations/xdto.rs::staged_xdto_conflicting_duplicate_is_refused_at_its_operation_without_writes
  - crates/unica-coder/src/infrastructure/native_operations/xdto.rs::staged_xdto_add_type_then_property_reads_prior_postimage
  - crates/unica-coder/src/infrastructure/native_operations/xdto.rs::staged_xdto_poisoned_second_op_publishes_nothing
  - crates/unica-coder/src/infrastructure/native_operations/xdto.rs::staged_xdto_dry_and_real_share_postimage_effects_and_revision
  - crates/unica-coder/src/infrastructure/daemon/xdto_can_tests.rs::forward_reference_is_validated_on_the_final_public_package_without_writes
  - crates/unica-coder/src/infrastructure/native_operations/xdto.rs::staged_xdto_rename_changes_only_names_and_keeps_pre_existing_findings
---

# Изменения пакета XDTO выполняются по порядку одной публикацией

Операция над пакетом XDTO видит результат предыдущих операций того же
запроса. Например, можно создать тип и сразу добавить ему свойство.
Ошибка последующей операции не оставляет частично записанный пакет
и указывает положение ошибочной операции.

Семантические ограничения пакета проверяются по итоговому образу всего
запроса относительно исходного пакета. Ссылка на тип, создаваемый позже
в том же запросе, допустима, если итоговый пакет корректен. Новые ошибки
итогового пакета блокируют публикацию и указывают исходную операцию;
ранее существовавшие ошибки не становятся новыми из-за смещения текста
или переименования свойства, к которому они относятся.
Проверки структуры XML, идентичности и допуска записи сохраняются на каждом
шаге.

Предпросмотр и применение строят одинаковый итоговый текст и события.
Предпросмотр сохраняет исходный файл; применение публикует пакет один раз.
Проверки проходят планировщик XDTO и публикацию через актора.

Точное повторное добавление типа или свойства не меняет пакет и ничего
не публикует. Добавление с тем же именем и иным содержимым отклоняется
с положением своей операции, и запрос ничего не записывает.
