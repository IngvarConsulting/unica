---
id: INV.SOURCE.PREDEFINED-ITEMS-ANSWER-BY-ADDRESS
status: active
governs: product
decision: DEC.2026-09-11.PREDEFINED-ITEMS-GET-A-READER
check: crates/unica-coder/src/infrastructure/v13_read/tests.rs::predefined_items_answer_by_address_with_the_count_from_the_reader
scope: [product, source]
---

# Предопределённый элемент читается по адресу, а счёт приходит от читателя

Вид-владелец предопределённых элементов объявляет ветвь `PredefinedItem` со
счётом из ответа читателя, а не из длины страницы. Элемент адресуется именем и
несёт собственные факты. Вид без этой коллекции ветви не получает: отсутствие
коллекции не равно нулю элементов.
