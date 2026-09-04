---
id: INV.APP.DOCUMENTATION-GET
status: superseded
governs: product
decision: DEC.2026-09-04.V0-13-LEGACY-BATCH-3
check: tests/ci/test_acceptance_scenarios.py::test_every_wire_answers_its_frozen_classes
scope: [app]
---

# Get пропускает поставщика без ответа и проецирует документ владельца

После поставщика, вернувшего `None`, следующий владелец локатора возвращает
документ с проверенными полями происхождения, идентичности и полного текста.

Правило снято вместе со своим предметом: выборка документа по локатору жила
в `unica.documentation.get`, а канонический `docs` отвечает поиском по тем же
корпусам. Отдельного входа за одним документом на поверхности нет.
