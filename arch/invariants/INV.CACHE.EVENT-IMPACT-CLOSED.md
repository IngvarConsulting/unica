---
id: INV.CACHE.EVENT-IMPACT-CLOSED
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check:
  - crates/unica-coder/src/domain/cache.rs::the_kind_list_covers_the_whole_enum
  - crates/unica-coder/src/domain/cache.rs::every_event_invalidates_at_least_one_cache
  - crates/unica-coder/src/domain/cache.rs::no_event_refreshes_a_cache_it_did_not_invalidate
  - crates/unica-coder/src/domain/cache.rs::from_events_unions_the_impact_of_every_event
  - crates/unica-coder/src/domain/cache.rs::no_events_leave_the_impact_empty
scope: [cache, product]
---

# Каждое типизированное событие имеет замкнутое влияние на кеш

Полный перечень вариантов доменных событий отображается хотя бы на одну
инвалидацию; eager refresh остаётся подмножеством инвалидированных кешей, а
влияние нескольких событий объединяется без потери.
