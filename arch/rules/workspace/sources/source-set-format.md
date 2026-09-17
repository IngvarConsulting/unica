---
id: INV.SOURCE.MULTI-FORMAT-WORKSPACE
check:
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
