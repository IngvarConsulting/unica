---
id: INV.PERF.SOURCE-SNAPSHOT-BYTE-BUDGET
status: superseded
governs: product
decision: DEC.2026-09-05.SOURCE-SNAPSHOT-PROVIDER-RETIRED
check: tests/ci/test_acceptance_scenarios.py::test_every_wire_answers_its_frozen_classes
scope: [product, source]
---

# Построение снимка не превышает его байтовый бюджет

Агрегатор прекращает построение до буферизации данных сверх объявленного
предела одного снимка.

Правило снято вместе с агрегатором снимков: канонический `view` не собирает
снимок источника в память целиком.
