---
id: INV.APP.VIEW-CURSOR-CAPACITY
check:
  - crates/unica-navigation/src/result_store.rs::every_promised_page_survives_a_chain_longer_than_128_pages
  - crates/unica-navigation/src/result_store.rs::new_snapshot_pressure_preserves_every_promised_chain
  - crates/unica-navigation/src/result_store.rs::concurrent_new_admissions_reserve_one_atomic_cumulative_budget
  - crates/unica-coder/src/infrastructure/daemon/server.rs::canonical_130_fresh_leases_share_one_payload_without_skipping_full_reads
  - crates/unica-navigation/src/result_store.rs::snapshot_admission_reserves_room_for_the_next_cursor
  - crates/unica-navigation/src/result_store.rs::many_small_items_are_charged_for_retained_heap_not_only_json
  - crates/unica-coder/src/application/v13/view.rs::snapshot_over_byte_quota_refuses_before_promising_a_continuation
  - crates/unica-coder/src/application/v13/view.rs::a_collection_longer_than_the_cursor_entry_limit_still_has_a_first_page
---

# Снимок чтения сохраняет повтор страниц в ограниченной памяти

Продолжение `view` сохраняет неизменяемый снимок коллекции при ограниченном
расходе памяти.
Учёт расхода памяти включает удерживаемые структуры и записи курсоров.
Число ещё не запрошенных страниц не занимает записи заранее.

Ограничение памяти сохраняет уже обещанное продолжение и точный повтор
страниц. Возраст курсора и обращения к другим результатам сами по себе
не разрешают удалить снимок. Допустимость ограничений задаёт
[правило обоснования ресурсов](../runtime/resource-limit-evidence.md),
а ограничивающих сроков — [правило таймеров](../runtime/timer-purpose.md).

Если новый снимок и его продолжение сохранить невозможно, ответ отказывает
до выдачи курсора. Повтор курсора возвращает ту же страницу и тот же
следующий курсор в пределах [процесса, сохранившего ответ](view-cursor-binding.md).
