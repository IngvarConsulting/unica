---
id: INV.APP.VIEW-CURSOR-CAPACITY
check:
  - crates/unica-coder/src/application/result_store.rs::long_chain_issues_one_cursor_at_a_time_and_reissues_an_evicted_successor
  - crates/unica-coder/src/application/result_store.rs::view_snapshot_byte_quota_and_lru_evict_old_chains
  - crates/unica-coder/src/application/result_store.rs::expired_view_cursor_releases_its_snapshot_charge
  - crates/unica-coder/src/application/result_store.rs::snapshot_admission_reserves_room_for_the_next_cursor
  - crates/unica-coder/src/application/result_store.rs::many_small_items_are_charged_for_retained_heap_not_only_json
  - crates/unica-coder/src/application/v13/view.rs::snapshot_over_byte_quota_refuses_before_promising_a_continuation
  - crates/unica-coder/src/application/result_store.rs::exact_revision_change_is_stale_but_tampering_and_expiry_are_invalid
---

# Снимок и курсоры чтения ограничены памятью и сроком жизни

Продолжение `view` хранит неизменяемый снимок коллекции только в процессе
сервера. Хранилище ограничивает учтённый объём снимков, число выданных
курсоров и срок их жизни; при нехватке места вытесняет давно не читанные
курсоры. Число ещё не запрошенных страниц не занимает записи заранее.

Если снимок не помещается в квоту, ответ не обещает продолжение. Повтор
сохранившегося курсора возвращает ту же страницу и тот же следующий курсор;
вытесненный или истёкший курсор отвечает `invalid_cursor`.
