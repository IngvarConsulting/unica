---
id: INV.APP.CONFIG-SNAPSHOT
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check: crates/unica-coder/src/domain/operational_config.rs::explicit_diagnostics_timeout_is_validated_and_overlaid_immutably
scope: [app]
---

# Оверлей конфигурации не меняет исходный снимок

Явный таймаут диагностики порождает новый проверенный снимок операционной
конфигурации, не меняя исходный; значение вне публичного диапазона отклоняется.
