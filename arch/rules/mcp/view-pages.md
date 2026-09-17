---
id: INV.APP.DEFERRED-MANIFEST
check:
  - crates/unica-coder/src/application/v13/view.rs::view_keeps_content_behind_explicit_body_and_paginates_whole_lines
  - crates/unica-coder/src/application/result_store.rs::cursor_chain_is_refused_before_it_can_exceed_the_entry_bound
---

# Коллекции view читаются ограниченными страницами

При чтении коллекции через `view` страница содержит не больше
запрошенного числа элементов. Для тела модуля элементом служит целая строка.
Если коллекция продолжается, ответ даёт непрозрачный курсор;
последняя страница курсора не содержит.

Хранилище отказывает в создании цепочки, которая превысит его вместимость.
Условия повторного использования курсора описаны
в [правиле продолжения чтения](view-cursor-binding.md).
