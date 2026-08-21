---
id: INV.PERF.SOURCE-SNAPSHOT-BYTE-BUDGET
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check: crates/unica-coder/src/infrastructure/platform_xml_resources.rs::source_resource_limits_and_cancellation_matrix_is_exact
scope: [product, source]
---

# Построение снимка не превышает его байтовый бюджет

Агрегатор прекращает построение до буферизации данных сверх объявленного
предела одного снимка.
