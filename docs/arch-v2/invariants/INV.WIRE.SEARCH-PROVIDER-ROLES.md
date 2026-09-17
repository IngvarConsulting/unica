---
id: INV.WIRE.SEARCH-PROVIDER-ROLES
status: active
governs: product
decision: DEC.2026-09-08.SEARCH-PROVIDER-ROLES
check: crates/unica-coder/src/application/v13/tool_catalog.rs::search_publishes_three_provider_roles_and_stays_literal_without_one
scope: [wire, product]
---

# Роль поиска — закрытый набор, и её отсутствие значимо

`search` публикует `role` перечислением ровно из трёх значений — `lexical`,
`symbol`, `semantic` — и без умолчания. Отсутствие роли не равно `lexical`:
оно означает поиск силами самой Unica, без внешнего провайдера и без его
цены. Четвёртая роль не добавляется молча: она меняет опубликованную
поверхность и приходит со своим решением.
