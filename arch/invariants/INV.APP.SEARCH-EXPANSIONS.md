---
id: INV.APP.SEARCH-EXPANSIONS
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check: crates/unica-coder/src/infrastructure/documentation_retrieval.rs::expansions_can_add_a_match_for_an_unmatched_query
scope: [app]
---

# Расширения могут добавить совпадение для исходно пустого запроса

Для русскоязычного запроса без попаданий английские расширения `group`, `table`
и `value` находят ожидаемый документ о таблицах значений.
