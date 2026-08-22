---
id: INV.CACHE.MUTATION-EVENT-COVERAGE
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check: crates/unica-coder/src/application/mod.rs::mutating_tools_have_typed_cache_event_or_explicit_non_cache_effect
scope: [app, cache, product]
---

# Мутаторы явно классифицированы по доменному событию

Нативная мутация объявляет тип события, metadata-координатор записывает его
транзакционно, а build/runtime либо объявляет событие, либо входит в закрытый
перечень операций без эффекта на предметный кеш. Новый неклассифицированный
мутатор останавливает проверку.
