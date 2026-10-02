---
id: INV.SOURCE.MULTI-FORMAT-WORKSPACE
check:
  - crates/unica-coder/src/infrastructure/project_sources.rs::a_declared_but_empty_source_set_is_declared_not_observed
  - crates/unica-coder/src/infrastructure/project_sources.rs::a_filled_source_set_is_supported_and_its_evidence_points_at_a_file
  - crates/unica-coder/src/infrastructure/tool_context.rs::native_platform_xml_source_format_public_gate_is_closed_over_public_operations
  - crates/unica-coder/src/infrastructure/project_sources.rs::detects_edt_configuration_and_platform_external_processor_source_sets
  - crates/unica-coder/src/infrastructure/project_sources.rs::controlled_discovery_accepts_uppercase_external_xml_extension
  - crates/unica-coder/src/infrastructure/project_sources.rs::conflicting_markers_inside_one_source_set_are_invalid_not_mixed
---

# У каждого набора исходников свой однозначный формат

Разные наборы одного проекта могут иметь разные форматы: например,
конфигурация в EDT, а внешние обработки и отчёты — в XML платформы.
Формат определяется отдельно для каждого набора.

Если внутри одного набора найдены противоречащие маркеры формата,
он получает состояние `Invalid`, а не смешанный формат.

Нативному обработчику XML нужен подходящий формат выбранного набора:
EDT и неоднозначный формат отклоняются. Проверка общего допуска обработчиков
покрывает физически адресованные операции; отказ нынешней MCP-поверхности
для проекта только с EDT описан [отдельным правилом](../../mcp/admission-refusal.md).

Объявленный пустой набор остаётся в карте проекта. Его состояние `declared`
отличает заявленный формат от наблюдённого: доказательство указывает на
`v8project.yaml`, а не на несуществующую выгрузку. При обнаружении выгрузки
поддерживаемого формата состояние становится `supported`, а доказательство
указывает на файл. Это наблюдение формата, не подтверждение исправности
всех исходников.
