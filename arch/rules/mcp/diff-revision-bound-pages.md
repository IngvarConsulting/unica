---
id: INV.WIRE.DIFF-REVISION-BOUND-PAGES
check:
  - crates/unica-coder/src/application/v13/diff.rs::diff_rejects_incomparable_logical_kinds
  - crates/unica-coder/src/application/v13/diff.rs::diff_limit_bounds_materialized_changes_and_issues_a_cursor
  - crates/unica-coder/src/application/v13/diff.rs::diff_cursor_is_bound_to_both_source_revisions
  - crates/unica-coder/src/application/v13/diff.rs::diff_cursor_cannot_be_replayed_for_another_question
  - crates/unica-coder/src/application/v13/diff.rs::diff_cursor_continues_from_the_bounded_change_offset
gap: https://github.com/IngvarConsulting/unica/issues/977
---

# Продолжение сравнения относится к той же паре ревизий

`diff` сравнивает узлы одинакового предметного вида. Размер страницы
ограничивает накопление изменений при обходе. Продолжение возвращает
следующие изменения того же сравнения в устойчивом порядке.

Курсор связан с обоими адресами, ревизиями, фильтром и размером страницы.
Изменение любой ревизии даёт `stale_cursor`, подмена вопроса — отказ.

Проверки проходят внутренний обработчик сравнения. Текущий публичный вызов
ещё отвергает курсор; подключение и сквозная проверка описаны в `gap`.
