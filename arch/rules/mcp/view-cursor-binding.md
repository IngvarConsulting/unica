---
id: INV.APP.DEFERRED-READ
check:
  - crates/unica-coder/src/application/result_store.rs::opaque_view_cursor_retry_is_idempotent_and_bound_to_the_complete_question
  - crates/unica-coder/src/application/result_store.rs::exact_revision_change_is_stale_but_tampering_and_expiry_are_invalid
  - crates/unica-coder/src/application/v13/view.rs::cursor_replay_is_bound_and_revision_change_is_stale
  - crates/unica-coder/src/application/v13/view.rs::retrying_the_same_cursor_returns_the_same_page_and_successor
  - crates/unica-coder/src/infrastructure/v13_read/tests.rs::cursor_retry_rejects_revision_change_during_role_canonicalization
---

# Курсор продолжает тот же вопрос на той же ревизии

Курсор `view` связан с адресом, видом представления, фильтром, набором
исходников и размером страницы. Подмена любого из этих условий
не позволяет получить чужой результат.
Курсор действует только в процессе, сохранившем результат.

После проверки вопроса и текущей ревизии страница берётся из сохранённого
результата. Повтор курсора возвращает ту же страницу и тот же следующий
курсор. Изменившаяся ревизия даёт `stale_cursor`; поддельный или истёкший
курсор — `invalid_cursor`.
