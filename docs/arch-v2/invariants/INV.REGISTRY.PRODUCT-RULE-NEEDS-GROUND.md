---
id: INV.REGISTRY.PRODUCT-RULE-NEEDS-GROUND
status: active
governs: process
decision: DEC.2026-08-19.PRODUCT-RECORD-IS-HISTORY
check: tests/arch/test_product_immutability.py::test_editing_a_product_rule_without_a_new_ground_is_caught
scope: [docs]
---

# Продуктовое правило меняется вместе с решением о причине

Инвариант и контракт со стороной `product` отличаются от целевой ветки только
если ссылаются на решение, заведённое той же правкой. Основание, записанное
раньше, этой перемены не предвидело и её не покрывает.
