---
id: INV.REGISTRY.RECIPROCAL-OWNERSHIP
status: superseded
governs: process
decision: DEC.2026-08-18.REGISTRY-SHAPE
check: tests/arch/test_registry.py::test_current_rule_owner_establishes_the_rule
scope: [docs]
---

# Решение и выведенное правило указывают друг на друга

Каждый инвариант и контракт входит в полный перечень своего решения, а каждый
элемент такого перечня ссылается обратно на то же решение.
