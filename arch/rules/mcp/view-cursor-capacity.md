---
id: INV.APP.VIEW-CURSOR-CAPACITY
check:
  - crates/unica-coder/src/application/result_store.rs::long_chain_issues_one_cursor_at_a_time_and_reissues_an_evicted_successor
  - crates/unica-coder/src/application/result_store.rs::view_snapshot_byte_quota_and_lru_evict_old_chains
  - crates/unica-coder/src/application/result_store.rs::expired_view_cursor_releases_its_snapshot_charge
  - crates/unica-coder/src/application/result_store.rs::snapshot_admission_reserves_room_for_the_next_cursor
  - crates/unica-coder/src/application/result_store.rs::many_small_items_are_charged_for_retained_heap_not_only_json
  - crates/unica-coder/src/application/v13/view.rs::snapshot_over_byte_quota_refuses_before_promising_a_continuation
  - crates/unica-coder/src/application/result_store.rs::source_change_preserves_the_snapshot_but_tampering_and_expiry_are_invalid
  - crates/unica-coder/tests/v13_search_integration.rs::public_view_pages_large_bsl_body_and_replays_disk_cursor
---

# Снимок чтения сохраняет повтор страниц без удержания большого Body в памяти

Продолжение `view` хранит неизменяемый снимок коллекции в памяти сервера
либо, для нефильтрованного `Body` модуля BSL, во временном файле, доступном только
процессу. Размер исходного файла не определяет допустимость такого снимка.
Хранилище ограничивает объём снимков в памяти, число выданных курсоров и
срок их жизни; давно не читанные курсоры вытесняются. Временный файл
удаляется после истечения срока последнего курсора, вытеснения или остановки
сервера. Число ещё не запрошенных страниц не занимает записи заранее.

Если снимок нельзя сохранить, ответ не обещает продолжение. Повтор
сохранившегося курсора возвращает ту же страницу и тот же следующий курсор;
вытесненный или истёкший курсор отвечает `invalid_cursor`.
