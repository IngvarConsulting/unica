---
id: INV.APP.DOCUMENTATION-GET
check:
  - crates/unica-coder/src/application/documentation.rs::get_skips_a_non_owner_and_projects_the_owners_document
  - crates/unica-coder/src/application/documentation.rs::a_locator_no_provider_owns_is_refused_naming_it
  - crates/unica-coder/src/application/documentation.rs::the_owners_failure_is_the_calls_failure
---

# Документ возвращает владеющий им поставщик справки

При чтении документа по локатору поставщик с ответом `None` пропускается:
это означает, что документ ему не принадлежит. Поиск продолжается
у следующего поставщика.

Документ найденного владельца доходит до результата с идентификатором,
заголовком, сигнатурой, полным текстом и сведениями о происхождении:
поставщиком, корпусом, видом источника, авторитетностью, языком и версией.

Если владельца нет, отказ называет локатор. Ошибка владельца сохраняет
причину отказа и не превращается в успешный документ.
