---
id: INV.SOURCE.READER-SELECTOR
status: superseded
governs: product
decision: DEC.2026-09-04.V0-13-LEGACY-BATCH-3
check: tests/ci/test_acceptance_scenarios.py::test_every_wire_answers_its_frozen_classes
scope: [source]
---

# Предметный читатель принимает ровно один селектор цели

Предметный читатель в переходном состоянии публикует логический селектор
`sourceSet` с `metadataPath` там, где инструмент его читает, и своё файловое
поле двумя взаимоисключающими ветвями схемы. Он принимает ровно один из них,
отклоняет оба стабильным `selector_conflict` до вызова обработчика и сохраняет
прежний отказ для вызова без единого селектора.

Правило снято вместе с мостом: у канонического чтения один адрес `at`, а не
две взаимоисключающие ветви селектора.
