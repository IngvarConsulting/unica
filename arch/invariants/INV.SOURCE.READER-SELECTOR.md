---
id: INV.SOURCE.READER-SELECTOR
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check: crates/unica-coder/src/application/tool_contracts.rs::bridged_reader_selector_schema_contract_is_complete
scope: [source]
---

# Предметный читатель принимает ровно один селектор цели

Предметный читатель в переходном состоянии публикует логический селектор
`sourceSet` с `metadataPath` там, где инструмент его читает, и своё файловое
поле двумя взаимоисключающими ветвями схемы. Он принимает ровно один из них,
отклоняет оба стабильным `selector_conflict` до вызова обработчика и сохраняет
прежний отказ для вызова без единого селектора.
