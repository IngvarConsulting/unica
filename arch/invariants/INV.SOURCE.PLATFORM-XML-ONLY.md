---
id: INV.SOURCE.PLATFORM-XML-ONLY
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check: crates/unica-coder/src/application/mod.rs::native_xml_metadata_tools_require_platform_xml_source_sets
scope: [source]
---

# Нативные операции с XML требуют platform XML

Нативная операция над метаданными сначала разрешает набор исходников с
`sourceFormat: platform_xml` и лишь затем трогает XML-файлы. EDT, недопустимый
или неоднозначный формат отклоняется типизированной ошибкой.
