---
id: INV.SOURCE.MODULE-SUMMARY
check:
  - crates/unica-coder/src/domain/module_projection.rs::serialized_module_projection_shape_is_stable
  - crates/unica-coder/src/infrastructure/bsl_module_projection.rs::every_approved_module_role_projects_without_a_parallel_role_registry
  - crates/unica-coder/src/infrastructure/bsl_module_projection.rs::valid_missing_physical_file_keeps_possible_events_but_no_source_projection
gap: https://github.com/IngvarConsulting/unica/issues/973
---

# Сводка модуля показывает ветви и их размер

Сводка объявляет `Method`, `Region`, `Interface`, `Event`, `Compilation`
и `Body` в этом порядке, с числом элементов каждой ветви. Методы и текст
тела в сводку не включаются. У допустимого модуля без файла нет фактов,
извлекаемых из текста, но применимые события платформы остаются доступны.

Проверки проходят модель сводки и построитель проекции модуля.
