---
id: INV.APP.V13-RUN-DICTIONARY
status: superseded
governs: product
decision: DEC.2026-09-02.DIRECTIONAL-RUNTIME-OPERATIONS
check: crates/unica-coder/src/application/v13/tool_catalog.rs::v13_run_dictionary_has_twelve_operations_without_query_execution
scope: [app, product, wire]
---

# Словарь Run v0.13 не содержит исполнение запросов

Каталог v0.13 публикует ровно двенадцать закрытых намерений `unica.run`.
`query.execute` среди них отсутствует и не имеет alias или capability-преемника
в v0.13.
