---
id: INV.SOURCE.READER-SELECTOR
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check:
  - crates/unica-coder/src/application/tool_contracts.rs::bridged_readers_publish_two_mutually_exclusive_selector_branches
  - crates/unica-coder/src/application/tool_contracts.rs::bridged_readers_refuse_two_selectors_at_once
  - crates/unica-coder/src/application/tool_contracts.rs::bridged_readers_still_refuse_a_call_with_no_selector
  - crates/unica-coder/src/application/tool_contracts.rs::bridged_readers_accept_either_selector_on_its_own
scope: [source]
---

# Предметный читатель принимает ровно один селектор цели

Предметный читатель в переходном состоянии публикует логический селектор
`sourceSet` с `metadataPath` там, где инструмент его читает, и своё файловое
поле двумя взаимоисключающими ветвями схемы. Он принимает ровно один из них,
отклоняет оба стабильным `selector_conflict` до вызова обработчика и сохраняет
прежний отказ для вызова без единого селектора.
