---
id: INV.SOURCE.ROLE-RIGHT-LOCAL-CHANGE
check:
  - crates/unica-coder/src/infrastructure/native_operations/role.rs::range_writer_uses_direct_children_and_preserves_unknown_xml_bom_and_eol
  - crates/unica-coder/src/infrastructure/native_operations/role.rs::range_writer_replaces_only_boolean_text_and_preserves_value_markup
  - crates/unica-coder/src/infrastructure/native_operations/role.rs::writer_inserts_with_existing_eol_and_data_processor_false_removes_whole_object
---

# Правка права сохраняет остальное описание роли

Писатель меняет выбранное право, сохраняя соседние права, неизвестные узлы,
шаблоны ограничений, начальный BOM и переводы строк. Разметка вокруг
изменяемого булева значения также сохраняется.

Исключения задаёт формат платформы: возврат к умолчанию удаляет запись
права с её ограничениями; `Use=false` для обработки `DataProcessor`
удаляет весь её блок. Проверки проходят внутренний писатель `Rights.xml`.
