---
id: INV.REGISTRY.REALIZATION-NAMED
status: active
governs: process
decision: DEC.2026-08-19.REALIZATION-AXIS
check: tests/arch/test_registry.py::test_a_realized_decision_names_evidence_that_exists
scope: [docs]
---

# Решение называет свидетельство реализации

Каждое решение несёт `realized`: путь к свидетельству в дереве или `null`.
Названное свидетельство существует, и символ после `::` находится в файле.
`null` допустим для ещё не реализованного направления; отсутствие пропа — нет.
