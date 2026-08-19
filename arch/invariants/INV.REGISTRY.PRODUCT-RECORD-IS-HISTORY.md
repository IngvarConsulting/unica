---
id: INV.REGISTRY.PRODUCT-RECORD-IS-HISTORY
status: active
governs: process
decision: DEC.2026-08-19.PRODUCT-RECORD-IS-HISTORY
check: tests/arch/test_product_immutability.py::test_editing_an_accepted_product_record_is_caught
scope: [docs]
---

# Продуктовая запись базы не редактируется

Запись со стороной `product`, присутствующая в целевой ветке, совпадает с ней
дословно или отличается только простановкой замены. Удаление такой записи
приравнивается к правке. Сторона определяется по базе.
