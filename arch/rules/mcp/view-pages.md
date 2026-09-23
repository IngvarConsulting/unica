---
id: INV.APP.DEFERRED-MANIFEST
check:
  - crates/unica-coder/src/application/v13/view.rs::view_keeps_content_behind_explicit_body_and_paginates_whole_lines
  - crates/unica-coder/src/application/v13/view.rs::a_collection_longer_than_the_cursor_entry_limit_still_has_a_first_page
---

# Коллекции view читаются ограниченными страницами

При чтении коллекции через `view` страница содержит не больше
запрошенного числа элементов. Для тела модуля элементом служит целая строка.
Если коллекция продолжается, ответ даёт непрозрачный курсор;
последняя страница курсора не содержит.

Страницы создаются по запросу: длинная цепочка сама по себе не мешает
получить первую страницу. Снимок коллекции должен помещаться в квоту
хранилища; иначе ответ явно отказывает до выдачи курсора.
Условия повторного использования курсора описаны
в [правиле продолжения чтения](view-cursor-binding.md).
