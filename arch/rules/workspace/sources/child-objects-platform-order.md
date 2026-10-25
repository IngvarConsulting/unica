---
id: INV.SOURCE.CHILD-OBJECTS-PLATFORM-ORDER
check:
  - crates/unica-source-designer/tests/platform/configuration_layout.rs::child_object_insertion_orders_bot_and_rejects_unknown_kinds
  - crates/unica-coder/src/infrastructure/native_operations/meta/edit.rs::child_objects_insert_into_the_platform_slot_of_their_owner_kind
  - crates/unica-coder/src/infrastructure/native_operations/apply_families/metadata.rs::member_and_template_adds_take_their_platform_slot_and_line_ending
  - crates/unica-forms-designer/tests/platform/form_codec.rs::form_registration_takes_the_platform_slot_of_its_owner_kind
---

# Новый элемент ChildObjects встаёт на место выгрузки платформы

Новый элемент `ChildObjects` владельца — реквизит, табличная часть, форма,
макет, команда и другие подчинённые объекты — записывается туда, куда его
поставила бы выгрузка платформы 8.3.27 для этого вида владельца. Выгрузка
группирует элементы по видам в порядке, своём для каждого вида владельца:
например, у документа формы идут перед табличными частями, у регистра
бухгалтерии первыми стоят измерения.

Внутри своего вида новый элемент встаёт после уже записанных элементов
этого вида. Существующие элементы не переставляются, даже если их порядок
отличается от порядка выгрузки. Для вида владельца без известного порядка
место выбирает операция записи.
