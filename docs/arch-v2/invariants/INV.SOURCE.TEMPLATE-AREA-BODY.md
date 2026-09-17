---
id: INV.SOURCE.TEMPLATE-AREA-BODY
status: active
governs: product
decision: DEC.2026-09-08.TEMPLATE-CELL-CONTENT
check:
  - crates/unica-coder/src/infrastructure/v13_read/tests.rs::template_area_cell_content_is_a_branch_read_only_when_its_address_is_asked
  - crates/unica-coder/src/infrastructure/v13_read/tests.rs::a_structural_template_read_never_serves_a_cached_payload_to_a_content_read
scope: [product, source]
---

# Содержимое области читается по её адресу и не переживает чужой разбор

Текст ячеек несёт только чтение `…Area.<Имя>.Body`. Область объявляет ветвь
`Body` с длиной, равной `props.contentCount`, и сама текста не несёт.

Признак содержимого входит в ключ кэша типизированных нагрузок. Предмет у
структурного чтения и у чтения содержимого один и тот же, поэтому без этого
различия отложенный структурный разбор был бы выдан на запрос текста, и ответ
оказался бы молча неполным. Проверка ставит структурное чтение перед чтением
содержимого в одной службе и требует полного текста.
