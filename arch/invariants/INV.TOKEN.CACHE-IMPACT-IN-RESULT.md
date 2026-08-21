---
id: INV.TOKEN.CACHE-IMPACT-IN-RESULT
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check: crates/unica-coder/src/application/mod.rs::every_mutator_defaults_to_preview_without_touching_storage
scope: [app, cache, product]
---

# Результат мутации сразу сообщает влияние на кеш

Тот же результат предпросмотра содержит режим кеша и списки событий и
инвалидаций, в том числе пустые, не требуя отдельного публичного вызова для
получения этой формы результата.
