---
id: INV.SOURCE.MODULE-SUMMARY
check:
  - crates/unica-coder/src/domain/module_projection.rs::serialized_module_projection_shape_is_stable
  - crates/unica-coder/src/application/v13/source_text.rs::response_budget_refuses_text_with_its_envelope_and_marks_oversized_branches
  - crates/unica-coder/src/application/v13/source_text.rs::overbudget_mark_and_refusal_share_one_boundary
  - crates/unica-coder/tests/v13_search_integration.rs::public_view_budget_belongs_to_the_client_and_the_summary_marks_the_body
  - crates/unica-coder/src/infrastructure/bsl_module_projection.rs::every_approved_module_role_projects_without_a_parallel_role_registry
  - crates/unica-coder/src/infrastructure/bsl_module_projection.rs::valid_missing_physical_file_keeps_possible_events_but_no_source_projection
gap: https://github.com/IngvarConsulting/unica/issues/973
---

# Сводка модуля показывает ветви и их размер

Сводка объявляет `Method`, `Region`, `Interface`, `Event`, `Compilation`
и `Body` в этом порядке, с числом элементов каждой ветви; у `Body` это число
строк текста. Ветвь `Body` модуля с файлом называет и размер текста
в байтах UTF-8 (`bytes`), а когда текст вместе с конвертом не поместится
в бюджет ответа клиента, помечена `overBudget`. Методы и текст тела в сводку
не включаются. У допустимого модуля без файла нет фактов,
извлекаемых из текста, но применимые события платформы остаются доступны.

Проверки проходят модель сводки и построитель проекции модуля.
