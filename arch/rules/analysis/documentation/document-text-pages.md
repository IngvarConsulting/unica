---
id: INV.APP.DOCUMENTATION-TEXT-PAGES
check:
  - crates/unica-coder/src/infrastructure/daemon/v13_documentation.rs::short_opened_document_preserves_the_whole_response
  - crates/unica-coder/src/infrastructure/daemon/v13_documentation.rs::opened_document_pages_reassemble_exact_utf8_text_and_replay
  - crates/unica-coder/src/infrastructure/daemon/v13_documentation.rs::prepared_document_locator_continues_across_calls_before_source_admission
  - crates/unica-coder/src/infrastructure/daemon/v13_documentation.rs::docs_locator_accepts_pagination_arguments
  - crates/unica-coder/src/infrastructure/daemon/v13_documentation.rs::document_over_transport_size_on_one_line_starts_with_a_useful_page
  - crates/unica-coder/src/infrastructure/daemon/v13_documentation.rs::oversized_document_metadata_refuses_before_issuing_a_cursor
---

# Страницы открытого документа сохраняют точный текст UTF-8

Если весь ответ помещается в обычную страницу, `unica.docs` возвращает полный
`document.text` одним ответом, без курсора и изменения прежней формы.
Длинный текст возвращается последовательными страницами того же документа.
Каждая страница содержит все сведения о происхождении
[документа владельца](document-owner.md) и точный фрагмент `document.text`.

`page.startByte`, `page.endByte` и `page.totalBytes` относятся к исходному
тексту UTF-8. Фрагменты в порядке курсора соединяются побайтно в полный текст
без нормализации переводов строк и пробелов. `page.stoppedBy` объясняет
остановку, а отсутствие курсора означает конец документа. `limit` считает
фрагменты: строку вместе с её переводом строки либо часть длинной строки
размером не более 16 КиБ. По умолчанию их не больше 20 на страницу,
максимум — 50.

Курсор сверяет локатор, выбранный вид источника, рабочую область и отпечаток
всего документа, включая текст и метаданные; изменившийся документ даёт
`stale_cursor`. Если владелец вернул текст, даже строка длиннее 8 МиБ начинается
полезной страницей. Сведения о документе не делятся: если они сами превышают
транспортный предел, отказ приходит до первого курсора.
Общие условия страницы задаёт [предел чтения](../../mcp/read-page-budget.md).
