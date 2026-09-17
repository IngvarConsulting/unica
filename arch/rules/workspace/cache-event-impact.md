---
id: INV.CACHE.EVENT-IMPACT-CLOSED
check:
  - crates/unica-coder/src/domain/cache.rs::the_kind_list_covers_the_whole_enum
  - crates/unica-coder/src/domain/cache.rs::every_event_invalidates_at_least_one_cache
  - crates/unica-coder/src/domain/cache.rs::no_event_refreshes_a_cache_it_did_not_invalidate
  - crates/unica-coder/src/domain/cache.rs::from_events_unions_the_impact_of_every_event
  - crates/unica-coder/src/domain/cache.rs::no_events_leave_the_impact_empty
---

# Влияние событий на кеш учитывается без потерь

Для каждого типа события изменения (`DomainEventKind`) Unica рассчитывает
непустой список устаревших кешей. Если событий несколько, списки объединяются:
влияние одного события не теряется из-за другого.

В план немедленного обновления входят только кеши из полученного списка.
Если событий нет, оба списка пусты.
