---
id: INV.APP.DEFERRED-READ
check:
  - crates/unica-coder/src/application/result_store.rs::opaque_view_cursor_retry_is_idempotent_and_bound_to_the_complete_question
  - crates/unica-coder/src/application/v13/view.rs::cursor_replay_keeps_the_issued_snapshot_after_source_change
  - crates/unica-coder/src/application/v13/view.rs::retrying_the_same_cursor_returns_the_same_page_and_successor
  - crates/unica-coder/src/infrastructure/v13_read/tests.rs::cursor_retry_uses_saved_role_results_without_recanonicalizing
gap: https://github.com/IngvarConsulting/unica/issues/1119
---

# Курсор продолжает сохранённый ответ на тот же вопрос

Курсор `view` связан с адресом вопроса, видом представления, набором
исходников и размером страницы. Подмена любого условия не позволяет
получить чужой результат. Курсор действует в процессе, сохранившем ответ.

Страница берётся из сохранённого результата без повторного чтения его
исходников и проверки общей ревизии. Изменение или удаление исходного файла
не меняет уже выданные страницы. Повтор курсора возвращает ту же страницу
и следующий курсор; поддельный курсор даёт `invalid_cursor`.
Новый вопрос читает исходники заново.

Сохранность продолжения задаёт [правило хранения снимка](view-cursor-capacity.md).
Действующее истечение курсоров остаётся в `gap`; гарантия не обещает
сохранение ответа после перезапуска процесса.
