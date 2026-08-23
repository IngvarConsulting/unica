---
id: INV.REGISTRY.RECIPROCAL-OWNERSHIP
status: active
governs: process
decision: DEC.2026-08-18.REGISTRY-SHAPE
check: tests/arch/test_registry.py::test_rule_decision_and_establishes_are_reciprocal
scope: [docs]
---

# Решение и выведенное правило указывают друг на друга

Каждый инвариант и контракт входит в полный перечень своего решения, а каждый
элемент такого перечня ссылается обратно на то же решение.
