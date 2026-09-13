---
id: INV.APP.SUPPORT-STATE
status: active
governs: product
decision: DEC.2026-09-04.V0-13-LEGACY-BATCH-3
check: tests/ci/test_acceptance_scenarios.py::test_every_wire_answers_its_frozen_classes
scope: [app]
---

# Чтение узнаёт поддержку по логической цели

Состояние поддержки приходит в ответ чтения по логическому адресу объекта или
подсистемы, а не по физическому пути marker-файла: канонический `view` кладёт
его в `props`, и корпус приёмки замораживает этот ответ на объектах поставщика.
