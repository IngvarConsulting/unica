---
id: INV.SOURCE.PREDEFINED-LOCAL-CHANGE
check:
  - crates/unica-coder/src/infrastructure/native_operations/meta/predefined.rs::update_preserves_unknown_nodes_using_a_namespace_declared_on_the_document_root
  - crates/unica-coder/src/infrastructure/native_operations/meta/predefined.rs::planner_clears_explicit_empty_structural_fields_and_repeats_as_a_noop
  - crates/unica-coder/src/infrastructure/native_operations/meta/predefined.rs::type_update_rejects_discarding_an_extension_in_a_removed_typed_branch
  - crates/unica-coder/src/infrastructure/native_operations/meta/predefined.rs::equivalent_simple_update_preserves_attribute_order_byte_for_byte
---

# Правка предопределённого элемента сохраняет неуказанное содержимое

Писатель меняет переданные поля элемента, сохраняя остальные поля,
неизвестные узлы и атрибуты. Явно пустые `accountingFlags` и
`extDimensionTypes` удаляют только поддержанные записи своих контейнеров.
Если замена типа уничтожила бы неизвестный узел, она отклоняется.
Повтор эквивалентного изменения не переписывает XML.

Проверки проходят писатель и планировщик `Predefined.xml`.
