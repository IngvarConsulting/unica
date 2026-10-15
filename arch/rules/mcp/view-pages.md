---
id: INV.APP.DEFERRED-MANIFEST
check:
  - crates/unica-coder/src/application/v13/view.rs::view_keeps_content_behind_explicit_branches_and_paginates_collections
  - crates/unica-coder/src/application/v13/view.rs::a_collection_longer_than_the_cursor_entry_limit_still_has_a_first_page
  - crates/unica-coder/src/application/v13/view.rs::module_text_is_returned_whole_beside_the_projected_node_without_pages
---

# Коллекции view читаются ограниченными страницами

При чтении коллекции через `view` страница содержит не больше
запрошенного числа элементов. Если коллекция продолжается, ответ даёт
непрозрачный курсор; последняя страница курсора не содержит.

Страницы создаются по запросу: длинная цепочка сама по себе не мешает
получить первую страницу. Снимок должен быть сохранён до выдачи курсора.

`Body` модуля BSL и `Method.<имя>.Body` коллекциями не являются и не
листаются; их выдачу задаёт [правило тела BSL](../workspace/sources/module-compilation-and-body.md).
Условия повторного использования курсора описаны
в [правиле продолжения чтения](view-cursor-binding.md).
