---
id: INV.WIRE.DIFF-REVISION-BOUND-PAGES
check:
  - crates/unica-coder/src/application/v13/diff.rs::diff_rejects_incomparable_logical_kinds
  - crates/unica-coder/src/application/v13/diff.rs::diff_limit_bounds_materialized_changes_and_issues_a_cursor
  - crates/unica-coder/src/application/v13/diff.rs::diff_cursor_replays_the_saved_comparison_after_sources_change
  - crates/unica-coder/src/application/v13/diff.rs::diff_cursor_cannot_be_replayed_for_another_question
  - crates/unica-coder/src/application/v13/diff.rs::diff_cursor_continues_from_the_bounded_change_offset
gap: https://github.com/IngvarConsulting/unica/issues/977
---

# Продолжение сравнения сохраняет прочитанную пару

`diff` сравнивает узлы одинакового предметного вида. Размер страницы
ограничивает накопление изменений при обходе. Продолжение возвращает
следующие изменения сохранённой пары в устойчивом порядке.

Курсор связан с обоими адресами, фильтром и размером страницы. Изменение
исходников не заменяет сохранённую пару; подмена вопроса даёт отказ.
Общие ревизии исходников не собираются и не проверяются.

Проверки проходят внутренний обработчик. Публичный вызов ещё отвергает
курсор; подключение и сквозная проверка описаны в `gap`.
