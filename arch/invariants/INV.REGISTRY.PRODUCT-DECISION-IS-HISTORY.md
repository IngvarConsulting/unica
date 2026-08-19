---
id: INV.REGISTRY.PRODUCT-DECISION-IS-HISTORY
status: active
governs: process
decision: DEC.2026-08-19.PRODUCT-RECORD-IS-HISTORY
check: tests/arch/test_product_immutability.py::test_editing_an_accepted_product_decision_is_caught
scope: [docs]
---

# Продуктовое решение из базы не правится

Решение со стороной `product`, присутствующее в целевой ветке, совпадает с ней
дословно или отличается только простановкой замены. Удаление приравнивается к
правке. Сторона определяется по базе.
