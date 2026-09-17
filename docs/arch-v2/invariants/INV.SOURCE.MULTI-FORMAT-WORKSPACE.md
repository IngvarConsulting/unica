---
id: INV.SOURCE.MULTI-FORMAT-WORKSPACE
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check: crates/unica-coder/src/infrastructure/project_sources.rs::detects_edt_configuration_and_platform_external_processor_source_sets
scope: [source]
---

# В рабочем пространстве действует несколько форматов

Одно рабочее пространство может содержать несколько наборов исходников с
разными действующими форматами, включая конфигурацию в формате EDT рядом с
внешними обработками и отчётами в формате platform XML.
