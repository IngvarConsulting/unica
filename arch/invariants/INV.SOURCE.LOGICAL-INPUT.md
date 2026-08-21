---
id: INV.SOURCE.LOGICAL-INPUT
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check: crates/unica-coder/src/application/tool_contracts.rs::logical_target_tools_do_not_publish_physical_identity_fields
scope: [source]
---

# Логическая цель не принимает физическую идентичность

Закрытая схема инструмента, принимающего `sourceSet` и `metadataPath` как
точную логическую цель без переходного моста, не публикует поля физического
пути и отклоняет их через `additionalProperties: false`.
