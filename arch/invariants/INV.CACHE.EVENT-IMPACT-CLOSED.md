---
id: INV.CACHE.EVENT-IMPACT-CLOSED
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check: crates/unica-coder/src/domain/cache.rs::typed_event_cache_impact_catalog_is_closed
scope: [cache, product]
---

# Каждое типизированное событие имеет замкнутое влияние на кеш

Полный перечень вариантов доменных событий отображается хотя бы на одну
инвалидацию; eager refresh остаётся подмножеством инвалидированных кешей, а
влияние нескольких событий объединяется без потери.
