---
id: INV.TOKEN.CACHE-IMPACT-IN-RESULT
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check: crates/unica-coder/src/application/mod.rs::preview_result_contains_cache_shape_without_second_call
scope: [app, cache, product]
---

# Результат мутации сразу сообщает влияние на кеш

Тот же результат предпросмотра содержит режим кеша и списки событий и
инвалидаций, в том числе пустые, не требуя отдельного публичного вызова для
получения этой формы результата.
