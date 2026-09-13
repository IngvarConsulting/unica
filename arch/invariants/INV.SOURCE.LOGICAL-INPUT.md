---
id: INV.SOURCE.LOGICAL-INPUT
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check: crates/unica-coder/src/application/tool_contracts.rs::logical_only_tool_schemas_match_exact_property_allowlists
scope: [source]
---

# Логическая цель не принимает физическую идентичность

Полные наборы свойств каждой логической схемы без переходного моста заданы
точными allowlist-списками и закрыты `additionalProperties: false`.
