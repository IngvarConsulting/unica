---
id: INV.REGISTRY.UNBUILT-SUPERSESSION
status: active
governs: process
decision: DEC.2026-08-31.HISTORICAL-RULE-OWNERSHIP
check: tests/arch/test_registry.py::test_decision_realized_is_status_dependent
scope: [ci, docs]
---

# Непостроенное заменённое решение не получает ложное evidence

Superseded decision может сохранять `realized: null`; отсутствие evidence
означает, что заменённое направление не было реализовано.
