---
id: INV.WIRE.SEARCH-CORPUS
status: active
governs: product
decision: DEC.2026-09-08.SEARCH-NAME-CORPUS
check: crates/unica-coder/src/application/v13/tool_catalog.rs::search_publishes_exactly_two_corpora_and_defaults_to_text
scope: [wire, product]
---

# Свод поиска — закрытый набор с текстовым умолчанием

`search` публикует `corpus` перечислением ровно из двух значений — `text` и
`names` — с умолчанием `text`. Третий свод не добавляется молча: он меняет
опубликованную поверхность и приходит со своим решением. Умолчание остаётся
текстовым, потому что таким `search` был до появления второго свода, и вызов
без `corpus` не меняет смысла.
