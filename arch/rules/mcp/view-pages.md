---
id: INV.APP.DEFERRED-MANIFEST
check:
  - crates/unica-coder/src/application/v13/view.rs::view_keeps_content_behind_explicit_body_and_paginates_whole_lines
  - crates/unica-coder/src/application/v13/view.rs::a_collection_longer_than_the_cursor_entry_limit_still_has_a_first_page
  - crates/unica-coder/tests/v13_search_integration.rs::public_view_starts_body_over_sixty_four_mib_and_splits_one_long_line
  - crates/unica-coder/src/application/v13/view.rs::disk_body_projection_without_items_returns_only_the_requested_node_sections
gap: https://github.com/IngvarConsulting/unica/issues/1119
---

# Коллекции view читаются ограниченными страницами

При чтении коллекции через `view` страница содержит не больше
запрошенного числа элементов. Для нефильтрованного `Body` модуля элементом служит строка,
пока она помещается в ответ. Строка, не помещающаяся в ответ целиком,
выдаётся последовательными фрагментами с исходным `line`, байтовым
`byteOffset` и признаком `endOfLine`; соединение `text` в порядке
смещений восстанавливает исходную строку без терминатора.
Если коллекция продолжается, ответ даёт непрозрачный курсор;
последняя страница курсора не содержит.

Страницы создаются по запросу: длинная цепочка сама по себе не мешает
получить первую страницу. Снимок должен быть сохранён до выдачи курсора;
для нефильтрованного `Body` BSL используется временное хранилище вместо удержания
всего текста в RAM.

Чтение `Body` с `filter.context` и `Method.<имя>.Body` пока строит полный
разбор модуля и не удовлетворяет этому правилу для больших файлов; разрыв
учтён в [#1119](https://github.com/IngvarConsulting/unica/issues/1119).
Условия повторного использования курсора описаны
в [правиле продолжения чтения](view-cursor-binding.md).
