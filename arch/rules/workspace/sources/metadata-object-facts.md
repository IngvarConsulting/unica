---
id: INV.SOURCE.METADATA-OBJECT-FACTS
check:
  - crates/unica-coder/src/infrastructure/daemon/metadata_can_tests.rs::canonical_bare_fill_value_is_a_diagnostic_not_an_absent_value
  - crates/unica-coder/src/infrastructure/meta_info_surface_tests.rs::info_marks_bare_fill_value_incomplete_with_diagnostic
  - crates/unica-coder/src/infrastructure/v13_read/tests/metadata_declaration_diagnostics.rs::malformed_standard_attributes_keep_the_cause_and_the_characteristic_sibling
  - crates/unica-coder/src/infrastructure/v13_read/tests/metadata_declaration_diagnostics.rs::malformed_characteristics_keep_the_cause_and_the_standard_attribute_sibling
  - crates/unica-coder/src/infrastructure/v13_read/tests/metadata_declaration_diagnostics.rs::malformed_standard_tabular_sections_keep_the_root_attribute_sibling
  - crates/unica-coder/src/infrastructure/v13_read/tests/metadata_declaration_diagnostics.rs::nested_standard_attributes_keep_their_standard_tabular_section_context
  - crates/unica-coder/src/infrastructure/v13_read/tests/metadata_declaration_diagnostics.rs::duplicate_standard_attributes_refuse_only_their_dependent_branch
  - crates/unica-coder/src/infrastructure/v13_read/tests/metadata_declaration_diagnostics.rs::duplicate_characteristics_refuse_only_their_dependent_branch
  - crates/unica-coder/src/infrastructure/v13_read/tests/metadata_declaration_diagnostics.rs::duplicate_standard_tabular_sections_refuse_only_their_dependent_branch
  - crates/unica-coder/src/infrastructure/v13_read/tests/metadata_declaration_diagnostics.rs::foreign_standard_attributes_refuse_only_their_dependent_branch
  - crates/unica-coder/src/infrastructure/v13_read/tests/metadata_declaration_diagnostics.rs::foreign_characteristics_refuse_only_their_dependent_branch
  - crates/unica-coder/src/infrastructure/v13_read/tests/metadata_declaration_diagnostics.rs::foreign_standard_tabular_sections_refuse_only_their_dependent_branch
  - crates/unica-coder/src/infrastructure/v13_read/tests/metadata_declaration_diagnostics.rs::similarly_named_property_diagnostics_do_not_poison_readable_declarations
  - crates/unica-coder/src/infrastructure/v13_read/tests/metadata_declaration_diagnostics.rs::valid_declarations_preserve_items_details_and_readonly_sources
  - crates/unica-coder/src/infrastructure/v13_read/tests/metadata_declaration_diagnostics.rs::projection_matches_canonical_segments_and_exact_xml_property_aliases
  - crates/unica-coder/src/infrastructure/v13_read/tests/metadata_declaration_diagnostics.rs::inapplicable_web_service_standard_attributes_ignore_a_metadata_container_alias
  - crates/unica-coder/src/infrastructure/v13_read/tests/metadata_declaration_diagnostics.rs::inapplicable_web_service_characteristics_ignore_a_metadata_container_alias
  - crates/unica-coder/src/infrastructure/v13_read/tests/metadata_declaration_diagnostics.rs::inapplicable_web_service_standard_tabular_sections_ignore_a_metadata_container_alias
  - crates/unica-coder/src/infrastructure/v13_read/tests/metadata_declaration_diagnostics.rs::inapplicable_web_service_standard_attributes_ignore_a_foreign_container_alias
  - crates/unica-coder/src/infrastructure/v13_read/tests/metadata_declaration_diagnostics.rs::inapplicable_web_service_characteristics_ignore_a_foreign_container_alias
  - crates/unica-coder/src/infrastructure/v13_read/tests/metadata_declaration_diagnostics.rs::inapplicable_web_service_standard_tabular_sections_ignore_a_foreign_container_alias
gap: https://github.com/IngvarConsulting/unica/issues/931
---

# Чтение сообщает состав объекта метаданных

Unica описывает объект по правилам поддерживаемой платформы и формата.
Отсутствие XML-секции само по себе не означает отсутствие реквизитов:
читатель учитывает стандартный состав, если его задаёт платформа.
Особенности записи XML не становятся отдельными состояниями объекта для ИИ.

Если состав нельзя достоверно установить, Unica сообщает причину,
а не выдаёт неизвестное за пустой список. Ошибка разбора не должна
теряться при подготовке ответа. Неподдерживаемая конструкция сама по себе
не доказывает повреждение исходников.

Внутренний читатель отличает отсутствие значения заполнения от повреждённой
записи: пустой нетипизированный `FillValue` даёт диагностику вместо молчаливого
ответа «значение не задано». Это проверяют внутренний читатель метаданных
и канонические `check` и `view` реквизита (#1933); весь разрыв текущего
публичного чтения проверка не закрывает.
