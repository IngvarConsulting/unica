---
id: INV.REGISTRY.HISTORICAL-ESTABLISHES
status: active
governs: process
decision: DEC.2026-08-31.HISTORICAL-RULE-OWNERSHIP
check: tests/arch/test_registry.py::test_current_rule_owner_establishes_the_rule
scope: [ci, docs]
---

# Текущий владелец устанавливает правило, исторический список не переписывается

Правило обязано входить в список устанавливаемых правил текущего
решения-владельца. Ссылка на то же правило в старом решении остаётся допустимой
историей.
