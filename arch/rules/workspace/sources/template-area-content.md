---
id: INV.SOURCE.TEMPLATE-AREA-BODY
check:
  - crates/unica-coder/src/infrastructure/v13_read/tests.rs::template_area_cell_content_is_a_branch_read_only_when_its_address_is_asked
  - crates/unica-coder/src/infrastructure/v13_read/tests.rs::a_structural_template_read_never_serves_a_cached_payload_to_a_content_read
---

# Текст ячеек области макета читается через Body

Узел области макета объявляет ветвь `Body` и число непустых ячеек
`props.contentCount`, но сам текст не возвращает. Чтение
`…Template.<Имя>.Area.<Имя>.Body` возвращает ячейки в исходном порядке:
`index`, `text` и `template` — признак подстановки параметров.

Предыдущее чтение структуры области не заменяет чтение текста неполным
результатом из кеша: запрос `Body` получает содержимое и после такого чтения.
