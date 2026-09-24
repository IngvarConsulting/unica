---
id: INV.APP.DOCUMENTATION-GET
check:
  - crates/unica-coder/src/application/documentation.rs::get_skips_a_non_owner_and_projects_the_owners_document
  - crates/unica-coder/src/application/documentation.rs::a_locator_no_provider_owns_is_refused_naming_it
  - crates/unica-coder/src/application/documentation.rs::the_owners_failure_is_the_calls_failure
  - crates/unica-coder/src/infrastructure/daemon/v13_documentation.rs::short_opened_document_preserves_the_whole_response
  - crates/unica-coder/src/infrastructure/daemon/v13_documentation.rs::opened_document_pages_reassemble_exact_utf8_text_and_replay
  - crates/unica-coder/src/infrastructure/daemon/v13_documentation.rs::prepared_document_locator_continues_across_calls_before_source_admission
---

# Документ возвращает владеющий им поставщик справки

При чтении документа по локатору поставщик с ответом `None` пропускается:
это означает, что документ ему не принадлежит. Поиск продолжается
у следующего поставщика.

Документ найденного владельца доходит до результата с идентификатором,
заголовком, сигнатурой, текстом и сведениями о происхождении: поставщиком,
корпусом, видом источника, авторитетностью, языком и версией. Если весь ответ
помещается в обычную страницу, `unica.docs` возвращает полный `document.text`
одним ответом, без курсора и изменения прежней формы.

Длинный текст возвращается последовательными страницами того же документа.
Каждая страница содержит все сведения о происхождении и точный фрагмент
`document.text`; `page.startByte`, `page.endByte` и `page.totalBytes` относятся
к исходному UTF-8 тексту. Фрагменты в порядке курсора соединяются побайтно
в полный текст без нормализации переводов строк и пробелов. `page.stoppedBy`
объясняет остановку, а отсутствие курсора означает конец документа. `limit`
считает фрагменты: строку вместе с её переводом строки либо часть длинной
строки размером не более 16 КиБ. По умолчанию их не больше 20 на страницу,
максимум — 50. Курсор сверяет локатор, выбранный вид источника, рабочую
область и отпечаток всего документа, включая текст и метаданные; изменившийся
документ даёт `stale_cursor`.

Если владельца нет, отказ называет локатор. Ошибка владельца сохраняет
причину отказа и не превращается в успешный документ.
